//! Create, update, delete, and move operations for Notes.
//! Wire format reverse-engineered from icloud.com web app HAR captures.

use serde_json::{json, Value};

use crate::cloudkit::{b64_encode_str, first_change_tag, CloudKitClient};
use crate::error::{Error, Result};

use super::cache::{NoteData, TRASH_FOLDER_ID};
use super::markdown;
use super::proto;
use super::sync::NotesSyncEngine;
use super::table;

const NOTE_UPDATE_LOOKUP_KEYS: &[&str] = &[
    "CreationDate",
    "ModificationDate",
    "TitleEncrypted",
    "SnippetEncrypted",
    "TextDataEncrypted",
    "Folder",
    "Folders",
    "FoldersModificationDate",
    "FirstAttachmentThumbnail",
    "FirstAttachmentUTIEncrypted",
    "TextDataAsset",
];

#[derive(Debug)]
struct ServerNoteForUpdate {
    change_tag: String,
    short_guid: Option<String>,
    creation_date: Option<Value>,
    folders_modified_date: Option<Value>,
    text_data_encrypted: String,
}

impl NotesSyncEngine {
    /// Build a closure that maps VFS note paths (e.g. `/Notes/Folder/Title.md`)
    /// back to `applenotes:note/UUID` URLs for the write path.
    fn build_reverse_link_resolver(&self) -> impl Fn(&str) -> Option<String> + '_ {
        // Pre-build lookup maps for O(1) resolution per link.
        let folder_by_name: std::collections::HashMap<&str, &str> = self
            .cache
            .folders
            .iter()
            .map(|(id, name)| (name.as_str(), id.as_str()))
            .collect();
        let note_by_folder_title: std::collections::HashMap<(&str, &str), &str> = self
            .cache
            .notes
            .iter()
            .filter(|(_, nd)| nd.is_active() && nd.folder_ref.is_some())
            .map(|(id, nd)| {
                (
                    (nd.folder_ref.as_deref().unwrap(), nd.title.as_str()),
                    id.as_str(),
                )
            })
            .collect();

        move |url: &str| -> Option<String> {
            let rest = url.strip_prefix("/Notes/")?;
            let slash = rest.find('/')?;
            let folder_name = &rest[..slash];
            let filename = &rest[slash + 1..];
            let title_stem = filename.strip_suffix(".md").unwrap_or(filename);
            let title = title_stem.replace('\u{2215}', "/");

            let folder_id = *folder_by_name.get(folder_name)?;
            let note_id = *note_by_folder_title.get(&(folder_id, title.as_str()))?;
            let owner = self.cache.owner_id.as_deref()?;
            Some(format!(
                "applenotes:note/{}?ownerIdentifier={}",
                note_id, owner
            ))
        }
    }

    fn resolve_with_tag(&self, partial: &str) -> Result<(String, String)> {
        let full = self
            .cache
            .find_note(partial)
            .ok_or_else(|| Error::Notes(format!("note '{partial}' not found")))?;
        let ct = self
            .cache
            .notes
            .get(&full)
            .and_then(|n| n.change_tag.clone())
            .ok_or_else(|| {
                Error::Notes(format!("missing change tag for '{partial}' — run sync"))
            })?;
        Ok((full, ct))
    }

    fn folder_ref(&self, folder_id: &str) -> Value {
        json!({
            "recordName": folder_id,
            "action": "VALIDATE",
            "zoneID": { "zoneName": self.ck.zone() }
        })
    }

    async fn lookup_note_for_update(&mut self, record_name: &str) -> Result<ServerNoteForUpdate> {
        let owner = self.owner_id().await?;
        let result = self
            .ck
            .lookup_records(&owner, &[record_name], NOTE_UPDATE_LOOKUP_KEYS)
            .await?;
        let rec = result["records"]
            .as_array()
            .and_then(|records| records.first())
            .ok_or_else(|| Error::Notes(format!("note '{record_name}' not found")))?;
        if let Some(code) = rec["serverErrorCode"].as_str() {
            let reason = rec["reason"].as_str().unwrap_or("");
            return Err(Error::Notes(format!("CloudKit {code}: {reason}")));
        }
        let change_tag = rec["recordChangeTag"]
            .as_str()
            .ok_or_else(|| Error::Notes(format!("missing change tag for '{record_name}'")))?
            .to_string();
        let fields = rec["fields"]
            .as_object()
            .ok_or_else(|| Error::Notes(format!("missing fields for note '{record_name}'")))?;
        Ok(ServerNoteForUpdate {
            change_tag,
            short_guid: rec["shortGUID"].as_str().map(str::to_owned),
            creation_date: fields
                .get("CreationDate")
                .and_then(|field| field.get("value"))
                .cloned(),
            folders_modified_date: fields
                .get("FoldersModificationDate")
                .and_then(|field| field.get("value"))
                .cloned(),
            text_data_encrypted: fields
                .get("TextDataEncrypted")
                .and_then(|field| field.get("value"))
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
        })
    }

    /// Create note with raw pre-encoded body (for testing exact HAR replay).
    pub async fn create_note_raw(
        &mut self,
        title: &str,
        snippet: &str,
        body_b64: &str,
        folder_name: &str,
    ) -> Result<String> {
        let _owner = self.owner_id().await?;
        let folder_id = self
            .cache
            .find_folder_by_name(folder_name)
            .ok_or_else(|| Error::Notes(format!("folder '{folder_name}' not found")))?;

        let record_name = uuid::Uuid::new_v4().to_string();
        let now = chrono::Utc::now().timestamp_millis();
        let fref = self.folder_ref(&folder_id);

        let op = json!({
            "operationType": "create",
            "record": {
                "recordType": "Note",
                "recordName": &record_name,
                "createShortGUID": true,
                "parent": { "recordName": &folder_id },
                "fields": {
                    "CreationDate": { "value": now },
                    "ModificationDate": { "value": now },
                    "TitleEncrypted": { "value": b64_encode_str(title) },
                    "SnippetEncrypted": { "value": b64_encode_str(snippet) },
                    "TextDataEncrypted": { "value": body_b64 },
                    "Folder": { "value": fref },
                    "Folders": { "value": [fref] },
                    "FirstAttachmentThumbnail": {},
                    "FirstAttachmentUTIEncrypted": {},
                    "TextDataAsset": {},
                }
            }
        });

        let result = modify_notes(&self.ck, vec![op]).await?;
        check_errors(&result)?;
        self.cache.ds.item_changed(record_name.clone());
        self.cache.notes.insert(
            record_name.clone(),
            NoteData {
                title: title.to_string(),
                snippet: snippet.to_string(),
                folder_ref: Some(folder_id),
                modified_ts: Some(now),
                deleted: false,
                change_tag: first_change_tag(&result),
                body_markdown: None,
                search_text: Some(super::sync::decode_search_text(body_b64)),
            },
        );
        Ok(record_name)
    }

    /// Create a new note from Markdown. Returns the record name.
    pub async fn create_note(&mut self, md: &str, folder_name: &str) -> Result<String> {
        let _owner = self.owner_id().await?;
        let folder_id = self
            .cache
            .find_folder_by_name(folder_name)
            .ok_or_else(|| Error::Notes(format!("folder '{folder_name}' not found")))?;

        let parsed = {
            let resolver = self.build_reverse_link_resolver();
            markdown::from_markdown_with_context(md, Some(&resolver))?
        };
        let doc = parsed.doc;
        let title = doc.text.lines().next().unwrap_or("Untitled").to_string();
        let snippet = doc
            .text
            .lines()
            .nth(1)
            .unwrap_or("")
            .chars()
            .take(200)
            .collect::<String>();
        let body_b64 = proto::encode_note_body(&doc)?;
        let title_b64 = b64_encode_str(&title);
        let snippet_b64 = b64_encode_str(&snippet);

        let record_name = uuid::Uuid::new_v4().to_string();
        let now = chrono::Utc::now().timestamp_millis();
        let fref = self.folder_ref(&folder_id);

        let op = json!({
            "operationType": "create",
            "record": {
                "recordType": "Note",
                "recordName": &record_name,
                "createShortGUID": true,
                "parent": { "recordName": &folder_id },
                "fields": {
                    "CreationDate": { "value": now },
                    "ModificationDate": { "value": now },
                    "TitleEncrypted": { "value": title_b64 },
                    "SnippetEncrypted": { "value": snippet_b64 },
                    "TextDataEncrypted": { "value": body_b64 },
                    "Folder": { "value": fref },
                    "Folders": { "value": [fref] },
                    "FirstAttachmentThumbnail": {},
                    "FirstAttachmentUTIEncrypted": {},
                    "TextDataAsset": {},
                }
            }
        });

        let result = modify_notes(&self.ck, vec![op]).await?;
        check_errors(&result)?;

        if !parsed.tables.is_empty() {
            self.create_table_attachments(&record_name, &parsed.tables, &doc, now)
                .await?;
        }

        let change_tag = first_change_tag(&result);
        self.cache.notes.insert(
            record_name.clone(),
            NoteData {
                title,
                snippet,
                folder_ref: Some(folder_id),
                modified_ts: Some(now),
                change_tag,
                body_markdown: Some(md.to_string()),
                search_text: Some(md.to_string()),
                ..Default::default()
            },
        );
        self.cache.ds.item_changed(record_name.clone());

        Ok(record_name)
    }

    /// Update a note's body from Markdown.
    pub async fn update_note(&mut self, partial: &str, md: &str) -> Result<()> {
        let _owner = self.owner_id().await?;
        let (full, _cached_ct) = self.resolve_with_tag(partial)?;
        let server = self.lookup_note_for_update(&full).await?;

        let nd = self
            .cache
            .notes
            .get(&full)
            .ok_or_else(|| Error::Notes("cache miss".into()))?;
        let folder_id = nd.folder_ref.clone().unwrap_or_default();

        let parsed = {
            let resolver = self.build_reverse_link_resolver();
            markdown::from_markdown_with_context(md, Some(&resolver))?
        };
        let doc = parsed.doc;
        let title = doc.text.lines().next().unwrap_or("Untitled").to_string();
        let snippet = doc
            .text
            .lines()
            .nth(1)
            .unwrap_or("")
            .chars()
            .take(200)
            .collect::<String>();
        let body_b64 = proto::encode_note_body_for_update(&server.text_data_encrypted, &doc)?;
        let title_b64 = b64_encode_str(&title);
        let snippet_b64 = b64_encode_str(&snippet);

        let now = chrono::Utc::now().timestamp_millis();
        let fref = self.folder_ref(&folder_id);
        let creation_date = server.creation_date.unwrap_or_else(|| Value::from(now));
        let folders_modified_date = server
            .folders_modified_date
            .unwrap_or_else(|| Value::from(now));

        let mut record = json!({
            "recordType": "Note",
            "recordName": &full,
            "recordChangeTag": server.change_tag,
            "parent": { "recordName": &folder_id },
            "fields": {
                "CreationDate": { "value": creation_date },
                "ModificationDate": { "value": now },
                "TitleEncrypted": { "value": title_b64 },
                "SnippetEncrypted": { "value": snippet_b64 },
                "TextDataEncrypted": { "value": body_b64 },
                "Folder": { "value": fref },
                "Folders": { "value": [fref] },
                "FoldersModificationDate": { "value": folders_modified_date },
                "FirstAttachmentThumbnail": {},
                "FirstAttachmentUTIEncrypted": {},
                "TextDataAsset": {},
            }
        });
        if let Some(short_guid) = server.short_guid {
            record["shortGUID"] = Value::String(short_guid);
        }

        let op = json!({
            "operationType": "update",
            "record": record
        });

        let result = modify_notes(&self.ck, vec![op]).await?;
        check_errors(&result)?;

        if !parsed.tables.is_empty() {
            self.create_table_attachments(&full, &parsed.tables, &doc, now)
                .await?;
        }

        if let Some(nd) = self.cache.notes.get_mut(&full) {
            nd.title = title;
            nd.snippet = snippet;
            nd.modified_ts = Some(now);
            nd.change_tag = first_change_tag(&result);
            nd.body_markdown = Some(md.to_string());
            nd.search_text = Some(md.to_string());
        }
        self.cache.ds.item_changed(full);
        Ok(())
    }

    /// Delete a note (moves to Trash folder, matching Apple Notes behavior).
    pub async fn delete_note(&mut self, partial: &str) -> Result<()> {
        let _owner = self.owner_id().await?;
        let (full, ct) = self.resolve_with_tag(partial)?;

        let nd = self
            .cache
            .notes
            .get(&full)
            .ok_or_else(|| Error::Notes("cache miss".into()))?;

        let trash_ref = self.folder_ref(TRASH_FOLDER_ID);
        let now = chrono::Utc::now().timestamp_millis();

        let mut fields = serde_json::Map::new();
        fields.insert("ModificationDate".into(), json!({"value": now}));
        fields.insert(
            "TitleEncrypted".into(),
            json!({"value": b64_encode_str(&nd.title)}),
        );
        fields.insert(
            "SnippetEncrypted".into(),
            json!({"value": b64_encode_str(&nd.snippet)}),
        );
        fields.insert("Folder".into(), json!({"value": trash_ref}));
        fields.insert("Folders".into(), json!({"value": [trash_ref]}));
        fields.insert("FoldersModificationDate".into(), json!({"value": now}));
        fields.insert("FirstAttachmentThumbnail".into(), json!({}));
        fields.insert("FirstAttachmentUTIEncrypted".into(), json!({}));
        fields.insert("TextDataAsset".into(), json!({}));

        let op = json!({
            "operationType": "update",
            "record": {
                "recordType": "Note",
                "recordName": &full,
                "recordChangeTag": ct,
                "parent": { "recordName": TRASH_FOLDER_ID },
                "fields": fields,
            }
        });

        let result = modify_notes(&self.ck, vec![op]).await?;
        check_errors(&result)?;
        self.cache.notes.remove(&full);
        self.cache.ds.item_deleted(full);
        Ok(())
    }

    /// Create Attachment records for tables referenced in the NoteDocument.
    async fn create_table_attachments(
        &self,
        note_record_name: &str,
        tables: &[table::TableData],
        doc: &super::models::NoteDocument,
        now: i64,
    ) -> Result<()> {
        let table_att_ids: Vec<String> = doc
            .runs
            .iter()
            .filter_map(|r| r.attachment.as_ref())
            .filter(|a| a.type_uti.as_deref() == Some("com.apple.notes.table"))
            .map(|a| a.identifier.clone())
            .collect();

        for (i, table_data) in tables.iter().enumerate() {
            let att_id = table_att_ids
                .get(i)
                .cloned()
                .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

            let mergeable_b64 = table::encode_table(table_data)?;
            let note_ref = json!({
                "recordName": note_record_name,
                "action": "VALIDATE",
                "zoneID": { "zoneName": self.ck.zone() }
            });

            let op = json!({
                "operationType": "create",
                "record": {
                    "recordType": "Attachment",
                    "recordName": &att_id,
                    "createShortGUID": true,
                    "parent": { "recordName": note_record_name },
                    "fields": {
                        "CreationDate": { "value": now },
                        "Note": { "value": note_ref },
                        "UTIEncrypted": { "value": b64_encode_str("com.apple.notes.table") },
                        "UTI": { "value": "com.apple.notes.table" },
                        "MinimumSupportedNotesVersion": { "value": 2 },
                        "TitleEncrypted": { "value": b64_encode_str("Table") },
                        "MergeableDataEncrypted": { "value": mergeable_b64 },
                        "MergeableDataAsset": {},
                        "EncryptedValues": {},
                        "EncryptedValuesAsset": {},
                    }
                }
            });

            let result = modify_notes(&self.ck, vec![op]).await?;
            check_errors(&result)?;
        }
        Ok(())
    }

    /// Move a note to a different folder.
    pub async fn move_note(&mut self, partial: &str, folder_name: &str) -> Result<()> {
        let _owner = self.owner_id().await?;
        let (full, ct) = self.resolve_with_tag(partial)?;
        let folder_id = self
            .cache
            .find_folder_by_name(folder_name)
            .ok_or_else(|| Error::Notes(format!("folder '{folder_name}' not found")))?;

        let nd = self
            .cache
            .notes
            .get(&full)
            .ok_or_else(|| Error::Notes("cache miss".into()))?;

        let fref = self.folder_ref(&folder_id);
        let now = chrono::Utc::now().timestamp_millis();

        let op = json!({
            "operationType": "update",
            "record": {
                "recordType": "Note",
                "recordName": &full,
                "recordChangeTag": ct,
                "parent": { "recordName": &folder_id },
                "fields": {
                    "ModificationDate": { "value": now },
                    "TitleEncrypted": { "value": b64_encode_str(&nd.title) },
                    "SnippetEncrypted": { "value": b64_encode_str(&nd.snippet) },
                    "Folder": { "value": fref },
                    "Folders": { "value": [fref] },
                    "FoldersModificationDate": { "value": now },
                    "FirstAttachmentThumbnail": {},
                    "FirstAttachmentUTIEncrypted": {},
                    "TextDataAsset": {},
                }
            }
        });

        let result = modify_notes(&self.ck, vec![op]).await?;
        check_errors(&result)?;

        if let Some(nd) = self.cache.notes.get_mut(&full) {
            nd.folder_ref = Some(folder_id);
            nd.change_tag = first_change_tag(&result);
        }
        self.cache.ds.item_changed(full);
        Ok(())
    }

    /// Create a new note folder (CloudKit `Folder` record).
    pub async fn create_folder(&mut self, name: &str) -> Result<String> {
        let _owner = self.owner_id().await?;
        if self.cache.find_folder_by_name(name).is_some() {
            return Err(Error::Notes(format!("folder '{name}' already exists")));
        }
        let record_name = uuid::Uuid::new_v4().to_string();
        let title_b64 = b64_encode_str(name);
        let mut fields = serde_json::Map::new();
        fields.insert("TitleEncrypted".into(), json!({"value": title_b64}));
        if let Some(notes_id) = self.cache.find_folder_by_name("Notes") {
            if let Some(parent) = self.cache.folder_parents.get(&notes_id) {
                fields.insert(
                    "ParentFolder".into(),
                    json!({"value": self.folder_ref(parent)}),
                );
            }
        }
        let op = json!({
            "operationType": "create",
            "record": {
                "recordType": "Folder",
                "recordName": &record_name,
                "createShortGUID": true,
                "fields": fields,
            }
        });
        let result = modify_notes(&self.ck, vec![op]).await?;
        check_errors(&result)?;
        self.cache
            .folders
            .insert(record_name.clone(), name.to_string());
        self.cache.ds.name_changed(record_name.clone());
        Ok(record_name)
    }

    /// Delete a folder that contains no notes (metadata only).
    pub async fn delete_folder(&mut self, folder_name: &str) -> Result<()> {
        let folder_id = self
            .cache
            .find_folder_by_name(folder_name)
            .ok_or_else(|| Error::Notes(format!("folder '{folder_name}' not found")))?;
        for nd in self.cache.notes.values() {
            if !nd.is_active() {
                continue;
            }
            if nd.folder_ref.as_deref() == Some(&folder_id) {
                return Err(Error::Notes(format!("folder '{folder_name}' is not empty")));
            }
        }
        let owner = self.owner_id().await?;
        let result = self
            .ck
            .lookup_records(&owner, &[&folder_id], &["TitleEncrypted"])
            .await?;
        let ct = result["records"]
            .as_array()
            .and_then(|recs| recs.first())
            .and_then(|r| r["recordChangeTag"].as_str())
            .ok_or_else(|| Error::Notes("cannot read folder change tag".into()))?
            .to_string();
        let op = json!({
            "operationType": "delete",
            "record": {
                "recordName": folder_id,
                "recordChangeTag": ct,
            }
        });
        let result = modify_notes(&self.ck, vec![op]).await?;
        check_errors(&result)?;
        self.cache.folders.remove(&folder_id);
        self.cache.folder_parents.remove(&folder_id);
        self.cache.ds.name_deleted(folder_id);
        Ok(())
    }
}

/// Notes-specific modify_records — matches icloud.com web app wire format:
/// no ownerRecordName in zoneID, no atomic flag.
async fn modify_notes(ck: &CloudKitClient, operations: Vec<Value>) -> Result<Value> {
    let path = format!(
        "database/1/{}/production/private/records/modify",
        ck.container()
    );
    let body = json!({
        "operations": operations,
        "zoneID": { "zoneName": ck.zone() },
    });
    ck.post(&path, &body).await
}

fn check_errors(result: &Value) -> Result<()> {
    crate::cloudkit::CloudKitClient::check_record_errors(result, Error::Notes)
}
