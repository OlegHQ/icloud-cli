//! In-memory cache for Notes sync state.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::store::{DirtyState, StoreCache};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct NoteData {
    pub title: String,
    #[serde(default)]
    pub snippet: String,
    #[serde(default, rename = "folder_ref")]
    pub folder_ref: Option<String>,
    #[serde(default, rename = "modified_ts")]
    pub modified_ts: Option<i64>,
    #[serde(default)]
    pub deleted: bool,
    #[serde(default, rename = "change_tag")]
    pub change_tag: Option<String>,
    #[serde(default, rename = "body_markdown")]
    pub body_markdown: Option<String>,
    #[serde(default, rename = "search_text")]
    pub search_text: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct NotesCache {
    pub notes: HashMap<String, NoteData>,
    pub folders: HashMap<String, String>,
    /// folder_id -> parent_folder_id (for tree display).
    #[serde(default, rename = "folder_parents")]
    pub folder_parents: HashMap<String, String>,
    #[serde(default, rename = "sync_token")]
    pub sync_token: Option<String>,
    #[serde(default, rename = "owner_id")]
    pub owner_id: Option<String>,
    #[serde(default, rename = "updated_at")]
    pub updated_at: Option<String>,
    /// Sync token as it was when this cache was loaded from disk (save-merge).
    #[serde(skip)]
    pub sync_token_at_load: Option<String>,
    #[serde(skip)]
    pub ds: DirtyState,
}

impl StoreCache for NotesCache {
    type Item = NoteData;
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
        &self.folders
    }
    fn names_mut(&mut self) -> &mut HashMap<String, String> {
        &mut self.folders
    }
    fn items(&self) -> &HashMap<String, Self::Item> {
        &self.notes
    }
    fn items_mut(&mut self) -> &mut HashMap<String, Self::Item> {
        &mut self.notes
    }
    fn label() -> &'static str {
        "notes"
    }
    fn meta_table() -> &'static str {
        "notes_meta"
    }
    fn names_table() -> &'static str {
        "notes_folders"
    }
    fn items_table() -> &'static str {
        "notes"
    }
    fn missing_version_is_empty() -> bool {
        true
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
    fn item_title(item: &NoteData) -> &str {
        &item.title
    }

    fn loaded_disk_sync_token(&self) -> Option<&str> {
        self.sync_token_at_load.as_deref()
    }

    fn set_loaded_disk_sync_token(&mut self, t: Option<String>) {
        self.sync_token_at_load = t;
    }

    fn extras(&self) -> Vec<(&'static str, String)> {
        if self.folder_parents.is_empty() {
            return vec![];
        }
        let json = serde_json::to_string(&self.folder_parents).unwrap_or_default();
        vec![("folder_parents", json)]
    }

    fn load_extras(&mut self, key: &str, value: &str) {
        if key == "folder_parents" {
            if let Ok(map) = serde_json::from_str(value) {
                self.folder_parents = map;
            }
        }
    }
}

impl NotesCache {
    pub fn find_folder_by_name(&self, name: &str) -> Option<String> {
        self.find_name(name)
    }

    pub fn find_note(&self, partial: &str) -> Option<String> {
        self.find_item(partial)
    }
}
