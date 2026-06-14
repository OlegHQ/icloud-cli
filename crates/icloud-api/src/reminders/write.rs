//! Mutation operations for Reminders (create, complete, delete, edit, list CRUD).

use serde_json::{json, Value};

use crate::cloudkit::first_change_tag;
use crate::error::{Error, Result};
use crate::title_doc::{encode_title, new_record_name, str_to_ts};

use super::cache::ReminderData;
use super::models::parse_priority;
use super::sync::SyncEngine;

impl SyncEngine {
    pub async fn add_reminder(
        &mut self,
        title: &str,
        list_name: &str,
        due: Option<&str>,
        priority: Option<&str>,
        notes: Option<&str>,
        parent_partial: Option<&str>,
    ) -> Result<String> {
        let owner = self.owner_id().await?;
        let list_id = self
            .cache
            .find_list_by_name(list_name)
            .ok_or_else(|| Error::Reminders(format!("list '{list_name}' not found")))?;
        let mut list_id_use = list_id.clone();
        let mut parent_full: Option<String> = None;
        if let Some(pp) = parent_partial {
            let p = self
                .cache
                .find_reminder(pp)
                .ok_or_else(|| Error::Reminders(format!("parent reminder '{pp}' not found")))?;
            parent_full = Some(p.clone());
            if let Some(d) = self.cache.reminders.get(&p) {
                if let Some(lr) = &d.list_ref {
                    list_id_use = lr.clone();
                }
            }
        }
        let priority_val = priority.and_then(parse_priority).unwrap_or(0);
        let op = build_create_op(
            title,
            &list_id_use,
            &owner,
            parent_full.as_deref(),
            due,
            priority_val,
            notes,
        )?;
        let record_name = op["record"]["recordName"]
            .as_str()
            .unwrap_or("")
            .to_string();
        let result = self.ck.modify_records(&owner, vec![op]).await?;
        check_ck_record_errors(&result)?;
        let mut rd = ReminderData {
            title: title.to_string(),
            completed: false,
            priority: priority_val,
            list_ref: Some(list_id_use),
            parent_ref: parent_full,
            ..Default::default()
        };
        if let Some(d) = due {
            rd.due = Some(d.to_string());
        }
        if let Some(n) = notes {
            rd.notes = Some(n.to_string());
        }
        rd.modified_ts = Some(chrono::Utc::now().timestamp_millis());
        rd.change_tag = first_change_tag(&result);
        let rn = record_name.clone();
        self.cache.reminders.insert(record_name.clone(), rd);
        self.cache.ds.item_changed(rn);
        Ok(record_name)
    }

    pub async fn add_reminders_batch(
        &mut self,
        titles: &[String],
        list_name: &str,
        parent_partial: Option<&str>,
    ) -> Result<Vec<String>> {
        let owner = self.owner_id().await?;
        let list_id = self
            .cache
            .find_list_by_name(list_name)
            .ok_or_else(|| Error::Reminders(format!("list '{list_name}' not found")))?;
        let mut parent_full: Option<String> = None;
        if let Some(pp) = parent_partial {
            let p = self
                .cache
                .find_reminder(pp)
                .ok_or_else(|| Error::Reminders(format!("parent reminder '{pp}' not found")))?;
            parent_full = Some(p.clone());
        }
        let mut ops = Vec::new();
        let mut names = Vec::new();
        for t in titles {
            let op = build_create_op(t, &list_id, &owner, parent_full.as_deref(), None, 0, None)?;
            let rn = op["record"]["recordName"]
                .as_str()
                .unwrap_or("")
                .to_string();
            names.push((rn, t.clone()));
            ops.push(op);
        }
        let result = self.ck.modify_records(&owner, ops).await?;
        check_ck_record_errors(&result)?;
        let now = chrono::Utc::now().timestamp_millis();
        let mut record_names: Vec<String> = Vec::with_capacity(names.len());
        for (rn, title) in names {
            let mut rd = ReminderData {
                title,
                list_ref: Some(list_id.clone()),
                parent_ref: parent_full.clone(),
                modified_ts: Some(now),
                ..Default::default()
            };
            if let Some(recs) = result["records"].as_array() {
                if let Some(ct) = recs
                    .iter()
                    .find(|r| r["recordName"].as_str() == Some(rn.as_str()))
                    .and_then(|r| r["recordChangeTag"].as_str())
                {
                    rd.change_tag = Some(ct.to_string());
                }
            }
            self.cache.reminders.insert(rn.clone(), rd);
            self.cache.ds.item_changed(rn.clone());
            record_names.push(rn);
        }
        Ok(record_names)
    }

    pub async fn complete_reminder(&mut self, partial: &str) -> Result<()> {
        let owner = self.owner_id().await?;
        let full = self
            .cache
            .find_reminder(partial)
            .ok_or_else(|| Error::Reminders(format!("reminder '{partial}' not found")))?;
        let rd = self
            .cache
            .reminders
            .get_mut(&full)
            .ok_or_else(|| Error::Reminders("cache miss".into()))?;
        let ct = rd.change_tag.clone().ok_or_else(|| {
            Error::Reminders(format!("missing change tag for '{partial}' — run sync"))
        })?;
        let now = chrono::Utc::now().timestamp_millis();
        let op = json!({
            "operationType": "update",
            "record": {
                "recordType": "Reminder",
                "recordName": full,
                "recordChangeTag": ct,
                "fields": {
                    "Completed": {"value": 1},
                    "CompletionDate": {"value": now},
                }
            }
        });
        let result = self.ck.modify_records(&owner, vec![op]).await?;
        check_ck_record_errors(&result)?;
        rd.completed = true;
        rd.completion_date = crate::title_doc::ts_to_str(now);
        rd.change_tag = first_change_tag(&result);
        self.cache.ds.item_changed(full);
        Ok(())
    }

    pub async fn delete_reminder(&mut self, partial: &str) -> Result<()> {
        let owner = self.owner_id().await?;
        let full = self
            .cache
            .find_reminder(partial)
            .ok_or_else(|| Error::Reminders(format!("reminder '{partial}' not found")))?;
        let ct = self
            .cache
            .reminders
            .get(&full)
            .and_then(|r| r.change_tag.clone())
            .ok_or_else(|| {
                Error::Reminders(format!("missing change tag for '{partial}' — run sync"))
            })?;
        let op = json!({
            "operationType": "delete",
            "record": {
                "recordName": full,
                "recordChangeTag": ct,
            }
        });
        let result = self.ck.modify_records(&owner, vec![op]).await?;
        check_ck_record_errors(&result)?;
        self.cache.reminders.remove(&full);
        self.cache.ds.item_deleted(full);
        Ok(())
    }

    /// Mark a reminder as incomplete (undo complete).
    pub async fn uncomplete_reminder(&mut self, partial: &str) -> Result<()> {
        let owner = self.owner_id().await?;
        let full = self
            .cache
            .find_reminder(partial)
            .ok_or_else(|| Error::Reminders(format!("reminder '{partial}' not found")))?;
        let rd = self
            .cache
            .reminders
            .get_mut(&full)
            .ok_or_else(|| Error::Reminders("cache miss".into()))?;
        let ct = rd.change_tag.clone().ok_or_else(|| {
            Error::Reminders(format!("missing change tag for '{partial}' — run sync"))
        })?;
        let op = json!({
            "operationType": "update",
            "record": {
                "recordType": "Reminder",
                "recordName": full,
                "recordChangeTag": ct,
                "fields": {
                    "Completed": {"value": 0},
                }
            }
        });
        let result = self.ck.modify_records(&owner, vec![op]).await?;
        check_ck_record_errors(&result)?;
        rd.completed = false;
        rd.completion_date = None;
        rd.change_tag = first_change_tag(&result);
        self.cache.ds.item_changed(full);
        Ok(())
    }

    /// Create a new reminder list.
    pub async fn create_list(&mut self, name: &str) -> Result<String> {
        let owner = self.owner_id().await?;
        let record_name = format!(
            "List/{}",
            uuid::Uuid::new_v4()
                .as_hyphenated()
                .to_string()
                .to_uppercase()
        );
        let op = build_create_list_op(&record_name, name);
        let result = self.ck.modify_records(&owner, vec![op]).await?;
        check_ck_record_errors(&result)?;
        self.cache
            .lists
            .insert(record_name.clone(), name.to_string());
        self.cache.ds.name_changed(record_name.clone());
        Ok(record_name)
    }

    /// Rename a reminder list.
    pub async fn rename_list(&mut self, name: &str, new_name: &str) -> Result<()> {
        let owner = self.owner_id().await?;
        let list_id = self
            .cache
            .find_list_by_name(name)
            .ok_or_else(|| Error::Reminders(format!("list '{name}' not found")))?;
        let result = self
            .ck
            .lookup_records(&owner, &[&list_id], &["Name", "TitleDocument"])
            .await?;
        let ct = result["records"]
            .as_array()
            .and_then(|recs| recs.first())
            .and_then(|r| r["recordChangeTag"].as_str())
            .ok_or_else(|| Error::Reminders("cannot read list change tag".into()))?
            .to_string();
        let op = json!({
            "operationType": "update",
            "record": {
                "recordType": "List",
                "recordName": &list_id,
                "recordChangeTag": ct,
                "fields": {
                    "Name": list_name_field(new_name),
                }
            }
        });
        let result = self.ck.modify_records(&owner, vec![op]).await?;
        check_ck_record_errors(&result)?;
        self.cache
            .lists
            .insert(list_id.clone(), new_name.to_string());
        self.cache.ds.name_changed(list_id);
        Ok(())
    }

    /// Delete a reminder list.
    pub async fn delete_list(&mut self, name: &str) -> Result<()> {
        let owner = self.owner_id().await?;
        let list_id = self
            .cache
            .find_list_by_name(name)
            .ok_or_else(|| Error::Reminders(format!("list '{name}' not found")))?;
        let result = self
            .ck
            .lookup_records(&owner, &[&list_id], &["Name"])
            .await?;
        let ct = result["records"]
            .as_array()
            .and_then(|recs| recs.first())
            .and_then(|r| r["recordChangeTag"].as_str())
            .ok_or_else(|| Error::Reminders("cannot read list change tag".into()))?
            .to_string();
        let op = json!({
            "operationType": "delete",
            "record": {
                "recordName": &list_id,
                "recordChangeTag": ct,
            }
        });
        let result = self.ck.modify_records(&owner, vec![op]).await?;
        check_ck_record_errors(&result)?;
        self.cache.lists.remove(&list_id);
        self.cache.ds.name_deleted(list_id);
        Ok(())
    }

    pub async fn edit_reminder(
        &mut self,
        partial: &str,
        title: Option<&str>,
        due: Option<&str>,
        clear_due: bool,
        notes: Option<&str>,
        priority: Option<&str>,
    ) -> Result<()> {
        if title.is_none() && due.is_none() && !clear_due && notes.is_none() && priority.is_none() {
            return Err(Error::Reminders(
                "no changes specified (title/due/clear-due/notes/priority)".into(),
            ));
        }
        let owner = self.owner_id().await?;
        let full = self
            .cache
            .find_reminder(partial)
            .ok_or_else(|| Error::Reminders(format!("reminder '{partial}' not found")))?;
        let ct = self
            .cache
            .reminders
            .get(&full)
            .and_then(|r| r.change_tag.clone())
            .ok_or_else(|| {
                Error::Reminders(format!("missing change tag for '{partial}' — run sync"))
            })?;
        let mut fields = serde_json::Map::new();
        if let Some(t) = title {
            let enc = encode_title(t)?;
            fields.insert("TitleDocument".into(), json!({"value": enc}));
            fields.insert("TitleDocumentAsset".into(), json!({"value": Value::Null}));
        }
        if clear_due {
            fields.insert("DueDate".into(), json!({}));
        } else if let Some(d) = due {
            let ts = str_to_ts(d).map_err(|e| Error::Reminders(e.to_string()))?;
            fields.insert("DueDate".into(), json!({"value": ts}));
        }
        if let Some(n) = notes {
            let enc = encode_title(n)?;
            fields.insert("NotesDocument".into(), json!({"value": enc}));
            fields.insert("NotesDocumentAsset".into(), json!({"value": Value::Null}));
        }
        if let Some(p) = priority {
            let pv =
                parse_priority(p).ok_or_else(|| Error::Reminders("invalid priority".into()))?;
            fields.insert("Priority".into(), json!({"value": pv}));
        }
        let op = json!({
            "operationType": "update",
            "record": {
                "recordType": "Reminder",
                "recordName": full,
                "recordChangeTag": ct,
                "fields": fields,
            }
        });
        let result = self.ck.modify_records(&owner, vec![op]).await?;
        check_ck_record_errors(&result)?;
        let rd = self
            .cache
            .reminders
            .get_mut(&full)
            .ok_or_else(|| Error::Reminders("cache miss".into()))?;
        if let Some(t) = title {
            rd.title = t.to_string();
        }
        if clear_due {
            rd.due = None;
        } else if let Some(d) = due {
            rd.due = Some(d.to_string());
        }
        if let Some(n) = notes {
            rd.notes = Some(n.to_string());
        }
        if let Some(p) = priority {
            rd.priority = parse_priority(p).unwrap_or(0);
        }
        rd.change_tag = first_change_tag(&result);
        rd.modified_ts = Some(chrono::Utc::now().timestamp_millis());
        self.cache.ds.item_changed(full);
        Ok(())
    }
}

fn list_name_field(name: &str) -> Value {
    json!({"value": name, "type": "STRING", "isEncrypted": true})
}

fn build_create_list_op(record_name: &str, name: &str) -> Value {
    json!({
        "operationType": "create",
        "record": {
            "recordType": "List",
            "recordName": record_name,
            "fields": {
                "Name": list_name_field(name),
                "Deleted": {"value": 0},
                "Imported": {"value": 0},
                "IsGroup": {"value": 0},
                "IsLinkedToAccount": {"value": 1},
                "ReminderIDs": {"value": "[]"},
                "ReminderIDsAsset": {"value": Value::Null},
                "BadgeEmblem": {"value": "default"},
                "Color": {
                    "value": "{\"daSymbolicColorName\":\"custom\",\"ckSymbolicColorName\":\"pink\",\"daHexString\":\"#EA426A\",\"red\":234,\"green\":66,\"blue\":106,\"alpha\":1,\"colorRGBSpace\":2}"
                },
            }
        }
    })
}

fn build_create_op(
    title: &str,
    list_id: &str,
    owner_id: &str,
    parent: Option<&str>,
    due: Option<&str>,
    priority: i32,
    notes: Option<&str>,
) -> Result<Value> {
    let encoded = encode_title(title)?;
    let empty_notes = encode_title("")?;
    let record_name = new_record_name();
    let now = chrono::Utc::now().timestamp_millis();
    let zone_ref = json!({
        "ownerRecordName": owner_id,
        "zoneName": "Reminders",
    });
    let mut fields = serde_json::Map::new();
    fields.insert("TitleDocument".into(), json!({"value": encoded}));
    fields.insert("TitleDocumentAsset".into(), json!({"value": Value::Null}));
    fields.insert("NotesDocument".into(), json!({"value": empty_notes}));
    fields.insert("NotesDocumentAsset".into(), json!({"value": Value::Null}));
    fields.insert("Completed".into(), json!({"value": 0}));
    fields.insert("AllDay".into(), json!({"value": 0}));
    fields.insert("CreationDate".into(), json!({"value": now}));
    fields.insert("Deleted".into(), json!({"value": 0}));
    fields.insert("Flagged".into(), json!({"value": 0}));
    fields.insert("Imported".into(), json!({"value": 0}));
    fields.insert("LastModifiedDate".into(), json!({"value": now}));
    if !list_id.is_empty() {
        fields.insert(
            "List".into(),
            json!({"value": {"recordName": list_id, "zoneID": zone_ref, "action": "VALIDATE"}}),
        );
    }
    if let Some(p) = parent {
        fields.insert(
            "ParentReminder".into(),
            json!({"value": {"recordName": p, "zoneID": zone_ref, "action": "VALIDATE"}}),
        );
    }
    if let Some(d) = due {
        let ts = str_to_ts(d).map_err(|e| Error::Reminders(e.to_string()))?;
        fields.insert("DueDate".into(), json!({"value": ts}));
    }
    if priority != 0 {
        fields.insert("Priority".into(), json!({"value": priority}));
    }
    if let Some(n) = notes {
        let enc = encode_title(n)?;
        fields.insert("NotesDocument".into(), json!({"value": enc}));
    }
    let mut record = json!({
        "recordType": "Reminder",
        "recordName": record_name,
        "fields": fields,
    });
    if !list_id.is_empty() {
        record["parent"] = json!({"recordName": list_id});
    }
    Ok(json!({
        "operationType": "create",
        "record": record,
    }))
}

fn check_ck_record_errors(result: &Value) -> Result<()> {
    crate::cloudkit::CloudKitClient::check_record_errors(result, Error::Reminders)
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;

    #[test]
    fn list_name_field_matches_icloud_string_shape() {
        let field = list_name_field("Groceries");

        assert_eq!(field["value"], "Groceries");
        assert_eq!(field["type"], "STRING");
        assert_eq!(field["isEncrypted"], true);
    }

    #[test]
    fn create_list_payload_uses_plain_encrypted_string_name() {
        let op = build_create_list_op("List/ABC", "The list");
        let fields = &op["record"]["fields"];

        assert_eq!(op["operationType"], "create");
        assert_eq!(op["record"]["recordType"], "List");
        assert_eq!(fields["Name"]["value"], "The list");
        assert_eq!(fields["Name"]["type"], "STRING");
        assert_eq!(fields["Name"]["isEncrypted"], true);
        assert_eq!(fields["ReminderIDs"]["value"], "[]");
        assert!(fields["ReminderIDsAsset"]["value"].is_null());
    }

    #[test]
    fn create_reminder_payload_matches_current_icloud_shape() {
        let op = build_create_op("buy milk", "List/ABC", "_defaultOwner", None, None, 0, None)
            .expect("payload");
        let record = &op["record"];
        let fields = &record["fields"];

        assert_eq!(record["recordType"], "Reminder");
        assert_eq!(record["parent"]["recordName"], "List/ABC");
        assert_eq!(fields["List"]["value"]["recordName"], "List/ABC");
        assert_eq!(
            fields["List"]["value"]["zoneID"]["ownerRecordName"],
            "_defaultOwner"
        );
        assert_eq!(fields["List"]["value"]["zoneID"]["zoneName"], "Reminders");
        assert_eq!(fields["List"]["value"]["action"], "VALIDATE");
        assert_eq!(fields["Completed"]["value"], 0);
        assert_eq!(fields["AllDay"]["value"], 0);
        assert_eq!(fields["Deleted"]["value"], 0);
        assert_eq!(fields["Flagged"]["value"], 0);
        assert_eq!(fields["Imported"]["value"], 0);
        assert!(fields["TitleDocumentAsset"]["value"].is_null());
        assert!(fields["NotesDocumentAsset"]["value"].is_null());

        let raw = base64::engine::general_purpose::STANDARD
            .decode(fields["TitleDocument"]["value"].as_str().expect("title"))
            .expect("base64 title");
        assert_eq!(raw.first(), Some(&0x78));
        assert_eq!(
            crate::title_doc::extract_title(fields["TitleDocument"]["value"].as_str().unwrap()),
            "buy milk"
        );
    }

    #[test]
    fn create_reminder_payload_keeps_parent_reminder_separate_from_record_parent() {
        let op = build_create_op(
            "subtask",
            "List/ABC",
            "_defaultOwner",
            Some("Reminder/PARENT"),
            None,
            0,
            None,
        )
        .expect("payload");
        let record = &op["record"];
        let fields = &record["fields"];

        assert_eq!(record["parent"]["recordName"], "List/ABC");
        assert_eq!(
            fields["ParentReminder"]["value"]["recordName"],
            "Reminder/PARENT"
        );
        assert_eq!(fields["ParentReminder"]["value"]["action"], "VALIDATE");
    }
}
