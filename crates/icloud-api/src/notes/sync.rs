//! CloudKit zone sync engine for Notes.

use serde_json::Value;

use crate::cloudkit::{
    ck_field_int, ck_field_ref, ck_field_timestamp, ensure_owner_id, CloudKitClient,
};
use crate::error::Result;
use crate::title_doc::ts_to_str;

use super::cache::{NoteData, NotesCache};
use super::markdown::{self, AttachmentContent};
use super::models::{Note, NoteFolder};
use super::proto;
use super::table;

const SYNC_DESIRED_KEYS: &[&str] = &[
    "TitleEncrypted",
    "SnippetEncrypted",
    "ModificationDate",
    "Deleted",
    "Folder",
    "ParentFolder",
];

const SYNC_RECORD_TYPES: &[&str] = &["Note", "Folder"];

const BODY_DESIRED_KEYS: &[&str] = &["TextDataEncrypted"];

pub struct NotesSyncEngine {
    pub ck: CloudKitClient,
    pub cache: NotesCache,
}

impl NotesSyncEngine {
    pub fn new(ck: CloudKitClient, cache: NotesCache) -> Self {
        Self { ck, cache }
    }

    pub async fn owner_id(&mut self) -> Result<String> {
        ensure_owner_id(&mut self.cache.owner_id, &self.ck).await
    }

    pub async fn sync(&mut self, force: bool) -> Result<()> {
        if force {
            self.cache = NotesCache::default();
            self.cache.ds.full_rewrite = true;
        }
        let owner_id = self.owner_id().await?;
        let old_token = self.cache.sync_token.clone();
        let mut token = self.cache.sync_token.clone();
        let records = self
            .ck
            .sync_zone_changes(
                &mut token,
                SYNC_DESIRED_KEYS,
                Some(SYNC_RECORD_TYPES),
                &owner_id,
            )
            .await?;
        if token != old_token {
            self.cache.ds.dirty = true;
        }
        self.cache.sync_token = token;
        self.process_records(records);
        Ok(())
    }

    /// Fetch the raw b64 body of a note.
    async fn fetch_b64(&mut self, record_name: &str) -> Result<String> {
        let owner = self.owner_id().await?;
        let result = self
            .ck
            .lookup_records(&owner, &[record_name], BODY_DESIRED_KEYS)
            .await?;
        let b64 = result["records"]
            .as_array()
            .and_then(|recs| recs.first())
            .and_then(|r| r["fields"]["TextDataEncrypted"]["value"].as_str())
            .unwrap_or("");
        Ok(b64.to_string())
    }

    /// Fetch all fields of a note record (for debugging).
    pub async fn fetch_raw(&mut self, record_name: &str) -> Result<serde_json::Value> {
        let owner = self.owner_id().await?;
        self.ck
            .lookup_records(
                &owner,
                &[record_name],
                &[
                    "TitleEncrypted",
                    "SnippetEncrypted",
                    "TextDataEncrypted",
                    "ModificationDate",
                    "Deleted",
                    "Folder",
                    "ParentFolder",
                    "Attachments",
                    "MinimumSupportedNotesVersion",
                ],
            )
            .await
    }

    /// Fetch the full body of a note and return it as Markdown.
    /// Resolves table attachments to render pipe tables inline.
    pub async fn fetch_body(&mut self, record_name: &str) -> Result<String> {
        let b64 = self.fetch_b64(record_name).await?;
        if b64.is_empty() {
            return Ok(String::new());
        }
        let doc = proto::decode_note_body(&b64)?;

        // Collect attachment IDs that are tables
        let table_ids: Vec<&str> = doc
            .runs
            .iter()
            .filter_map(|r| r.attachment.as_ref())
            .filter(|a| a.type_uti.as_deref() == Some("com.apple.notes.table"))
            .map(|a| a.identifier.as_str())
            .collect();

        let mut attachments = std::collections::HashMap::new();

        // Fetch table data for each table attachment
        if !table_ids.is_empty() {
            let owner = self.owner_id().await?;
            let result = self
                .ck
                .lookup_records(&owner, &table_ids, &["MergeableDataEncrypted"])
                .await?;
            if let Some(recs) = result["records"].as_array() {
                for rec in recs {
                    let rn = rec["recordName"].as_str().unwrap_or("");
                    if rec["serverErrorCode"].as_str().is_some() {
                        continue;
                    }
                    let md_b64 = rec["fields"]["MergeableDataEncrypted"]["value"]
                        .as_str()
                        .unwrap_or("");
                    if !md_b64.is_empty() {
                        if let Ok(td) = table::decode_table(md_b64) {
                            attachments.insert(rn.to_string(), AttachmentContent::Table(td));
                        }
                    }
                }
            }
        }

        // Collect image/file attachment info
        for run in &doc.runs {
            if let Some(ref att) = run.attachment {
                if attachments.contains_key(&att.identifier) {
                    continue; // already resolved (table)
                }
                let uti = att.type_uti.as_deref().unwrap_or("unknown");
                if uti.starts_with("public.png")
                    || uti.starts_with("public.jpeg")
                    || uti.starts_with("public.image")
                    || uti.starts_with("public.tiff")
                    || uti.starts_with("public.heic")
                {
                    attachments.insert(
                        att.identifier.clone(),
                        AttachmentContent::Image(uti.to_string()),
                    );
                } else if uti != "com.apple.notes.table" {
                    attachments.insert(
                        att.identifier.clone(),
                        AttachmentContent::File(uti.to_string()),
                    );
                }
            }
        }

        Ok(markdown::to_markdown_with_attachments(&doc, &attachments))
    }

    /// Fetch the full body and dump raw protobuf attribute runs (for debugging).
    pub async fn debug_body(&mut self, record_name: &str) -> Result<String> {
        let b64 = self.fetch_b64(record_name).await?;
        if b64.is_empty() {
            return Ok("(empty body)".into());
        }
        proto::debug_dump(&b64)
    }

    pub fn get_notes(&self) -> Vec<Note> {
        let mut out = Vec::new();
        for (id, nd) in &self.cache.notes {
            if nd.deleted {
                continue;
            }
            let folder_name = nd
                .folder_ref
                .as_ref()
                .and_then(|fr| self.cache.folders.get(fr))
                .cloned()
                .unwrap_or_else(|| "Notes".into());
            out.push(Note {
                id: id.clone(),
                title: nd.title.clone(),
                snippet: if nd.snippet.is_empty() {
                    None
                } else {
                    Some(nd.snippet.clone())
                },
                folder_id: nd.folder_ref.clone(),
                folder_name,
                modified: nd.modified_ts.and_then(ts_to_str),
                body: None,
            });
        }
        out
    }

    pub fn get_folders(&self) -> Vec<NoteFolder> {
        self.cache
            .folders
            .iter()
            .map(|(id, name)| NoteFolder {
                id: id.clone(),
                name: name.clone(),
                parent_id: self.cache.folder_parents.get(id).cloned(),
            })
            .collect()
    }

    fn process_records(&mut self, records: Vec<Value>) {
        for rec in records {
            let Some(map) = rec.as_object() else {
                continue;
            };
            let record_name = map.get("recordName").and_then(|v| v.as_str()).unwrap_or("");
            let record_type = map.get("recordType").and_then(|v| v.as_str()).unwrap_or("");
            let deleted = map
                .get("deleted")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let fields = map
                .get("fields")
                .and_then(|v| v.as_object())
                .cloned()
                .unwrap_or_default();

            match record_type {
                "Folder" => {
                    if deleted {
                        self.cache.folders.remove(record_name);
                        self.cache.folder_parents.remove(record_name);
                        self.cache.ds.name_deleted(record_name.to_string());
                    } else {
                        let title = field_b64_text(&fields, "TitleEncrypted");
                        if !title.is_empty() {
                            self.cache.folders.insert(record_name.to_string(), title);
                            self.cache.ds.name_changed(record_name.to_string());
                        }
                        if let Some(parent) = ck_field_ref(&fields, "ParentFolder") {
                            self.cache
                                .folder_parents
                                .insert(record_name.to_string(), parent);
                        }
                    }
                }
                "Note" => {
                    if deleted {
                        self.cache.notes.remove(record_name);
                        self.cache.ds.item_deleted(record_name.to_string());
                    } else {
                        let title = field_b64_text(&fields, "TitleEncrypted");
                        let snippet = field_b64_text(&fields, "SnippetEncrypted");
                        let folder_ref = ck_field_ref(&fields, "Folder");
                        let modified_ts = ck_field_timestamp(&fields, "ModificationDate");
                        let change_tag = map
                            .get("recordChangeTag")
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string());
                        let is_deleted = ck_field_int(&fields, "Deleted") != 0;

                        let nd = NoteData {
                            title: if title.is_empty() {
                                "(untitled)".into()
                            } else {
                                title
                            },
                            snippet,
                            folder_ref,
                            modified_ts,
                            deleted: is_deleted,
                            change_tag,
                        };
                        self.cache.notes.insert(record_name.to_string(), nd);
                        self.cache.ds.item_changed(record_name.to_string());
                    }
                }
                _ => {}
            }
        }
    }
}

fn field_b64_text(fields: &serde_json::Map<String, Value>, key: &str) -> String {
    let b64 = fields
        .get(key)
        .and_then(|f| f.get("value"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    proto::decode_b64_text(b64)
}
