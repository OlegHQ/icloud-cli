//! Create, update, delete, and move operations for Notes.
//! Wire format reverse-engineered from icloud.com web app HAR captures.

use serde_json::{json, Value};

use crate::cloudkit::{b64_encode_str, first_change_tag, CloudKitClient};
use crate::error::{Error, Result};

use super::cache::NoteData;
use super::markdown;
use super::proto;
use super::sync::NotesSyncEngine;
use super::table;

impl NotesSyncEngine {
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
        Ok(record_name)
    }

    /// Create a new note from Markdown. Returns the record name.
    pub async fn create_note(&mut self, md: &str, folder_name: &str) -> Result<String> {
        let _owner = self.owner_id().await?;
        let folder_id = self
            .cache
            .find_folder_by_name(folder_name)
            .ok_or_else(|| Error::Notes(format!("folder '{folder_name}' not found")))?;

        let parsed = markdown::from_markdown(md)?;
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
                ..Default::default()
            },
        );
        self.cache.ds.item_changed(record_name.clone());

        Ok(record_name)
    }

    /// Update a note's body from Markdown.
    pub async fn update_note(&mut self, partial: &str, md: &str) -> Result<()> {
        let _owner = self.owner_id().await?;
        let (full, ct) = self.resolve_with_tag(partial)?;

        let nd = self
            .cache
            .notes
            .get(&full)
            .ok_or_else(|| Error::Notes("cache miss".into()))?;
        let folder_id = nd.folder_ref.clone().unwrap_or_default();

        let parsed = markdown::from_markdown(md)?;
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

        let now = chrono::Utc::now().timestamp_millis();
        let fref = self.folder_ref(&folder_id);

        let op = json!({
            "operationType": "update",
            "record": {
                "recordType": "Note",
                "recordName": &full,
                "recordChangeTag": ct,
                "parent": { "recordName": &folder_id },
                "fields": {
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
            self.create_table_attachments(&full, &parsed.tables, &doc, now)
                .await?;
        }

        if let Some(nd) = self.cache.notes.get_mut(&full) {
            nd.title = title;
            nd.snippet = snippet;
            nd.modified_ts = Some(now);
            nd.change_tag = first_change_tag(&result);
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

        let trash_id = "TrashFolder-CloudKit";
        let trash_ref = self.folder_ref(trash_id);
        let now = chrono::Utc::now().timestamp_millis();

        let mut fields = serde_json::Map::new();
        fields.insert(
            "ModificationDate".into(),
            json!({"value": now}),
        );
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
        fields.insert(
            "FoldersModificationDate".into(),
            json!({"value": now}),
        );
        fields.insert("FirstAttachmentThumbnail".into(), json!({}));
        fields.insert("FirstAttachmentUTIEncrypted".into(), json!({}));
        fields.insert("TextDataAsset".into(), json!({}));

        let op = json!({
            "operationType": "update",
            "record": {
                "recordType": "Note",
                "recordName": &full,
                "recordChangeTag": ct,
                "parent": { "recordName": trash_id },
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
