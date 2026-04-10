//! CloudKit zone sync engine for Notes.

use std::collections::{HashMap, HashSet};

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
const SEARCH_LOOKUP_BATCH_SIZE: usize = 100;

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
        if let Some(body) = self
            .cache
            .notes
            .get(record_name)
            .and_then(|note| note.body_markdown.clone())
        {
            return Ok(body);
        }

        let b64 = self.fetch_b64(record_name).await?;
        if b64.is_empty() {
            if let Some(note) = self.cache.notes.get_mut(record_name) {
                note.body_markdown = Some(String::new());
                note.search_text = Some(String::new());
                self.cache.ds.item_changed(record_name.to_string());
            }
            return Ok(String::new());
        }
        let doc = proto::decode_note_body(&b64)?;

        // Partition attachment IDs into tables vs non-tables in a single pass
        let mut table_ids = Vec::new();
        let mut non_table_ids = Vec::new();
        for run in &doc.runs {
            if let Some(ref att) = run.attachment {
                if att.type_uti.as_deref() == Some("com.apple.notes.table") {
                    table_ids.push(att.identifier.as_str());
                } else {
                    non_table_ids.push(att.identifier.as_str());
                }
            }
        }

        let mut attachments = std::collections::HashMap::new();
        let owner = self.owner_id().await?;

        // Fetch table data
        if !table_ids.is_empty() {
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

        let mut att_titles = std::collections::HashMap::new();
        if !non_table_ids.is_empty() {
            if let Ok(result) = self
                .ck
                .lookup_records(&owner, &non_table_ids, &["TitleEncrypted"])
                .await
            {
                if let Some(recs) = result["records"].as_array() {
                    for rec in recs {
                        let rn = rec["recordName"].as_str().unwrap_or("");
                        if rec["serverErrorCode"].as_str().is_some() {
                            continue;
                        }
                        let title_b64 = rec["fields"]["TitleEncrypted"]["value"]
                            .as_str()
                            .unwrap_or("");
                        if !title_b64.is_empty() {
                            let title = proto::decode_b64_text(title_b64);
                            if !title.is_empty() {
                                att_titles.insert(rn.to_string(), title);
                            }
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
                let title = att_titles.get(&att.identifier).cloned();
                if markdown::is_image_uti(uti) {
                    attachments.insert(
                        att.identifier.clone(),
                        AttachmentContent::Image(uti.to_string(), title),
                    );
                } else if uti != "com.apple.notes.table" {
                    attachments.insert(
                        att.identifier.clone(),
                        AttachmentContent::File(uti.to_string(), title),
                    );
                }
            }
        }

        // Build link resolver: map applenotes:note/UUID → /Notes/Folder/Title.md
        let link_resolver = |url: &str| -> Option<String> {
            let rest = url.strip_prefix("applenotes:note/")?;
            let uuid = rest.split('?').next()?;
            let nd = self.cache.notes.get(uuid)?;
            let folder_name = nd
                .folder_ref
                .as_ref()
                .and_then(|fr| self.cache.folders.get(fr))?;
            let stem = markdown::title_to_filename_stem(&nd.title);
            Some(format!("/Notes/{}/{}.md", folder_name, stem))
        };

        let markdown =
            markdown::to_markdown_with_attachments(&doc, &attachments, Some(&link_resolver));

        if let Some(note) = self.cache.notes.get_mut(record_name) {
            note.body_markdown = Some(markdown.clone());
            note.search_text = Some(markdown.clone());
            self.cache.ds.item_changed(record_name.to_string());
        }

        Ok(markdown)
    }

    pub async fn hydrate_search_texts(&mut self, record_names: &[String]) -> Result<()> {
        let note_ids: Vec<String> = record_names
            .iter()
            .filter(|record_name| {
                self.cache
                    .notes
                    .get(record_name.as_str())
                    .is_some_and(|note| !note.deleted && note.search_text.is_none())
            })
            .cloned()
            .collect();

        if note_ids.is_empty() {
            return Ok(());
        }

        let owner = self.owner_id().await?;
        for chunk in note_ids.chunks(SEARCH_LOOKUP_BATCH_SIZE) {
            let record_names: Vec<&str> = chunk.iter().map(String::as_str).collect();
            let result = self
                .ck
                .lookup_records(&owner, &record_names, BODY_DESIRED_KEYS)
                .await?;

            let mut resolved = HashMap::with_capacity(chunk.len());
            if let Some(records) = result["records"].as_array() {
                for record in records {
                    let Some(record_name) = record["recordName"].as_str() else {
                        continue;
                    };
                    if record["serverErrorCode"].as_str().is_some() {
                        continue;
                    }
                    let search_text = record["fields"]["TextDataEncrypted"]["value"]
                        .as_str()
                        .map(decode_search_text)
                        .unwrap_or_default();
                    resolved.insert(record_name.to_string(), search_text);
                }
            }

            let resolved_names: HashSet<String> = resolved.keys().cloned().collect();
            for (record_name, search_text) in resolved {
                if let Some(note) = self.cache.notes.get_mut(&record_name) {
                    note.search_text = Some(search_text);
                    self.cache.ds.item_changed(record_name);
                }
            }

            for record_name in chunk {
                if resolved_names.contains(record_name) {
                    continue;
                }
                if let Some(note) = self.cache.notes.get_mut(record_name) {
                    note.search_text = Some(String::new());
                    self.cache.ds.item_changed(record_name.clone());
                }
            }
        }

        Ok(())
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

                        let cached_body = self
                            .cache
                            .notes
                            .get(record_name)
                            .and_then(|existing| {
                                (existing.modified_ts == modified_ts
                                    && existing.folder_ref == folder_ref
                                    && existing.deleted == is_deleted)
                                    .then(|| existing.body_markdown.clone())
                            })
                            .flatten();
                        let cached_search_text = self
                            .cache
                            .notes
                            .get(record_name)
                            .and_then(|existing| {
                                (existing.modified_ts == modified_ts
                                    && existing.folder_ref == folder_ref
                                    && existing.deleted == is_deleted)
                                    .then(|| {
                                        existing
                                            .search_text
                                            .clone()
                                            .or_else(|| existing.body_markdown.clone())
                                    })
                            })
                            .flatten();

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
                            body_markdown: cached_body,
                            search_text: cached_search_text,
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

pub(crate) fn decode_search_text(b64: &str) -> String {
    if b64.is_empty() {
        return String::new();
    }

    match proto::decode_note_body(b64) {
        Ok(document) => document.text.replace('\u{fffc}', " ").trim().to_string(),
        Err(_) => proto::decode_b64_text(b64),
    }
}

#[cfg(test)]
mod tests {
    use super::decode_search_text;
    use crate::notes::models::NoteDocument;
    use crate::notes::proto::encode_note_body;

    #[test]
    fn decodes_note_body_into_search_text() {
        let body = encode_note_body(&NoteDocument {
            text: "alpha\nbeta\n".into(),
            runs: vec![],
        })
        .unwrap();
        assert_eq!(decode_search_text(&body), "alpha\nbeta");
    }
}
