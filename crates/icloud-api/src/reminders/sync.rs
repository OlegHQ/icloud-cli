use serde_json::Value;

use crate::cloudkit::{
    ck_field_int, ck_field_int64, ck_field_ref, ck_field_string, ensure_owner_id, CloudKitClient,
};
use crate::error::Result;
use crate::title_doc::{extract_title, ts_to_str};

/// Decode a list name: current iCloud records use encrypted STRING; older CLI
/// writes used base64 BYTES, so keep that fallback for already-synced records.
fn decode_name_or_title(raw_name: &str, fields: &serde_json::Map<String, Value>) -> String {
    if !raw_name.is_empty() {
        let name_type = fields
            .get("Name")
            .and_then(|f| f.get("type"))
            .and_then(|v| v.as_str());
        if name_type == Some("BYTES") {
            if let Ok(bytes) =
                base64::Engine::decode(&base64::engine::general_purpose::STANDARD, raw_name)
            {
                if let Ok(s) = std::str::from_utf8(&bytes) {
                    let s = s.trim();
                    if !s.is_empty() {
                        return s.to_string();
                    }
                }
            }
        }
        return raw_name.to_string();
    }
    extract_title(&ck_field_string(fields, "TitleDocument"))
}

use super::cache::{ReminderData, RemindersCache};

pub struct SyncEngine {
    pub ck: CloudKitClient,
    pub cache: RemindersCache,
}

impl SyncEngine {
    pub fn new(ck: CloudKitClient, cache: RemindersCache) -> Self {
        Self { ck, cache }
    }

    pub async fn owner_id(&mut self) -> Result<String> {
        ensure_owner_id(&mut self.cache.owner_id, &self.ck).await
    }

    pub async fn sync(&mut self, force: bool) -> Result<()> {
        if force {
            self.cache = RemindersCache::default();
            self.cache.ds.full_rewrite = true;
        }
        let owner_id = self.owner_id().await?;
        const KEYS: &[&str] = &[
            "TitleDocument",
            "NotesDocument",
            "Name",
            "Completed",
            "CompletionDate",
            "DueDate",
            "List",
            "Deleted",
            "Priority",
            "ParentReminder",
        ];
        let old_token = self.cache.sync_token.clone();
        let mut token = self.cache.sync_token.clone();
        let records = self
            .ck
            .sync_zone_changes(&mut token, KEYS, None, &owner_id)
            .await?;
        if token != old_token {
            self.cache.ds.dirty = true;
        }
        self.cache.sync_token = token;
        self.process_records(records);
        Ok(())
    }

    pub fn get_reminders(&self, include_completed: bool) -> Vec<super::models::Reminder> {
        let mut out = Vec::new();
        for (id, data) in &self.cache.reminders {
            if !include_completed && data.completed {
                continue;
            }
            let list_name = data
                .list_ref
                .as_ref()
                .and_then(|lr| self.cache.lists.get(lr))
                .cloned()
                .unwrap_or_else(|| "?".into());
            out.push(super::models::Reminder {
                id: id.clone(),
                title: data.title.clone(),
                completed: data.completed,
                completion_date: data.completion_date.clone(),
                due: data.due.clone(),
                priority: data.priority,
                notes: data.notes.clone(),
                list_ref: data.list_ref.clone(),
                list_name,
                parent_ref: data.parent_ref.clone(),
                modified_ts: data.modified_ts,
            });
        }
        out
    }

    pub fn get_lists(&self) -> Vec<super::models::ReminderList> {
        self.cache
            .lists
            .iter()
            .map(|(id, name)| super::models::ReminderList {
                id: id.clone(),
                name: name.clone(),
            })
            .collect()
    }

    fn process_records(&mut self, records: Vec<Value>) {
        for rec in records {
            let Some(map) = rec.as_object() else { continue };
            let record_name = map.get("recordName").and_then(|v| v.as_str()).unwrap_or("");
            let record_type = map.get("recordType").and_then(|v| v.as_str()).unwrap_or("");
            let mut deleted = map
                .get("deleted")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let fields = map
                .get("fields")
                .and_then(|v| v.as_object())
                .cloned()
                .unwrap_or_default();

            if ck_field_int(&fields, "Deleted") != 0 {
                deleted = true;
            }

            match record_type {
                "ReminderList" | "List" => {
                    if deleted {
                        self.cache.lists.remove(record_name);
                        self.cache.ds.name_deleted(record_name.to_string());
                    } else {
                        let raw_name = ck_field_string(&fields, "Name");
                        let title = decode_name_or_title(&raw_name, &fields);
                        if !title.is_empty() {
                            self.cache.lists.insert(record_name.to_string(), title);
                            self.cache.ds.name_changed(record_name.to_string());
                        }
                    }
                }
                "Reminder" => {
                    if deleted {
                        self.cache.reminders.remove(record_name);
                        self.cache.ds.item_deleted(record_name.to_string());
                    } else {
                        let mut title = extract_title(&ck_field_string(&fields, "TitleDocument"));
                        if title.is_empty() {
                            title = "(untitled)".into();
                        }
                        let due = ck_field_int64(&fields, "DueDate").and_then(ts_to_str);
                        let completion =
                            ck_field_int64(&fields, "CompletionDate").and_then(ts_to_str);
                        let list_ref = ck_field_ref(&fields, "List");
                        let parent_ref = ck_field_ref(&fields, "ParentReminder");
                        let notes_raw = extract_title(&ck_field_string(&fields, "NotesDocument"));
                        let notes = if notes_raw.is_empty() {
                            None
                        } else {
                            Some(notes_raw)
                        };
                        let priority = ck_field_int(&fields, "Priority");
                        let change_tag = map
                            .get("recordChangeTag")
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string());
                        let modified_ts = map
                            .get("modified")
                            .and_then(|m| m.get("timestamp"))
                            .and_then(|v| v.as_f64())
                            .map(|f| f as i64);

                        let rd = ReminderData {
                            title,
                            completed: ck_field_int(&fields, "Completed") != 0,
                            completion_date: completion,
                            due,
                            priority,
                            notes,
                            list_ref,
                            parent_ref,
                            modified_ts,
                            change_tag,
                        };
                        self.cache.reminders.insert(record_name.to_string(), rd);
                        self.cache.ds.item_changed(record_name.to_string());
                    }
                }
                _ => {}
            }
        }
    }
}
