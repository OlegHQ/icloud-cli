use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::store::{DirtyState, StoreCache};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ReminderData {
    pub title: String,
    pub completed: bool,
    #[serde(default, rename = "completion_date")]
    pub completion_date: Option<String>,
    #[serde(default)]
    pub due: Option<String>,
    #[serde(default)]
    pub priority: i32,
    #[serde(default)]
    pub notes: Option<String>,
    #[serde(default, rename = "list_ref")]
    pub list_ref: Option<String>,
    #[serde(default, rename = "parent_ref")]
    pub parent_ref: Option<String>,
    #[serde(default, rename = "modified_ts")]
    pub modified_ts: Option<i64>,
    #[serde(default, rename = "change_tag")]
    pub change_tag: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RemindersCache {
    pub reminders: HashMap<String, ReminderData>,
    pub lists: HashMap<String, String>,
    #[serde(default, rename = "sync_token")]
    pub sync_token: Option<String>,
    #[serde(default, rename = "owner_id")]
    pub owner_id: Option<String>,
    #[serde(default, rename = "updated_at")]
    pub updated_at: Option<String>,
    #[serde(skip)]
    pub sync_token_at_load: Option<String>,
    #[serde(skip)]
    pub ds: DirtyState,
}

impl StoreCache for RemindersCache {
    type Item = ReminderData;
    fn sync_token(&self) -> Option<&str> {
        self.sync_token.as_deref()
    }
    fn set_sync_token(&mut self, t: Option<String>) {
        self.sync_token = t;
    }
    fn owner_id(&self) -> Option<&str> {
        self.owner_id.as_deref()
    }
    fn set_owner_id(&mut self, id: Option<String>) {
        self.owner_id = id;
    }
    fn names(&self) -> &HashMap<String, String> {
        &self.lists
    }
    fn names_mut(&mut self) -> &mut HashMap<String, String> {
        &mut self.lists
    }
    fn items(&self) -> &HashMap<String, Self::Item> {
        &self.reminders
    }
    fn items_mut(&mut self) -> &mut HashMap<String, Self::Item> {
        &mut self.reminders
    }
    fn label() -> &'static str {
        "reminders"
    }
    fn meta_table() -> &'static str {
        "meta"
    }
    fn names_table() -> &'static str {
        "lists"
    }
    fn items_table() -> &'static str {
        "reminders"
    }
    fn missing_version_is_empty() -> bool {
        false
    }

    fn updated_at_str(&self) -> Option<&str> {
        self.updated_at.as_deref()
    }
    fn set_updated_at_str(&mut self, ts: Option<String>) {
        self.updated_at = ts;
    }
    fn ds(&self) -> &DirtyState {
        &self.ds
    }
    fn ds_mut(&mut self) -> &mut DirtyState {
        &mut self.ds
    }
    fn item_title(item: &ReminderData) -> &str {
        &item.title
    }

    fn loaded_disk_sync_token(&self) -> Option<&str> {
        self.sync_token_at_load.as_deref()
    }

    fn set_loaded_disk_sync_token(&mut self, t: Option<String>) {
        self.sync_token_at_load = t;
    }
}

impl RemindersCache {
    pub fn find_list_by_name(&self, name: &str) -> Option<String> {
        self.find_name(name)
    }

    pub fn find_reminder(&self, partial: &str) -> Option<String> {
        self.find_item(partial)
    }
}
