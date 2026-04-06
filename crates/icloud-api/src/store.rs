//! Generic redb-backed store shared by Reminders and Notes.
//! Both modules provide a `StoreCache` impl; all DB logic lives here once.

use std::collections::{HashMap, HashSet};
use std::fs::{self, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};

use fs2::FileExt;
use redb::{Database, ReadableTable, TableDefinition, TableError};
use serde::{Deserialize, Serialize};

use crate::error::Result;

const KEY_SCHEMA_VERSION: &str = "schema_version";
const KEY_SYNC_TOKEN: &str = "sync_token";
const KEY_OWNER_ID: &str = "owner_id";
const KEY_UPDATED_AT: &str = "updated_at";
const EXPECTED_SCHEMA: &str = "1";

fn is_missing_table(err: &TableError) -> bool {
    matches!(err, TableError::TableDoesNotExist(_))
}

/// Tracks which items/names were modified or deleted since the last save.
#[derive(Debug, Clone, Default)]
pub struct DirtyState {
    pub dirty: bool,
    /// When true, the DB was freshly created (force sync / first run) — write everything.
    pub full_rewrite: bool,
    pub dirty_items: HashSet<String>,
    pub dirty_names: HashSet<String>,
    pub deleted_items: Vec<String>,
    pub deleted_names: Vec<String>,
}

impl DirtyState {
    pub fn item_changed(&mut self, id: String) {
        self.dirty = true;
        self.dirty_items.insert(id);
    }

    pub fn item_deleted(&mut self, id: String) {
        self.dirty = true;
        self.dirty_items.remove(&id);
        self.deleted_items.push(id);
    }

    pub fn name_changed(&mut self, id: String) {
        self.dirty = true;
        self.dirty_names.insert(id);
    }

    pub fn name_deleted(&mut self, id: String) {
        self.dirty = true;
        self.dirty_names.remove(&id);
        self.deleted_names.push(id);
    }

    pub fn reset(&mut self) {
        self.dirty = false;
        self.full_rewrite = false;
        self.dirty_items.clear();
        self.dirty_names.clear();
        self.deleted_items.clear();
        self.deleted_names.clear();
    }
}

/// Cache type that can be persisted into a `RedbStore`.
pub trait StoreCache: Default {
    type Item: Serialize + for<'de> Deserialize<'de>;

    fn sync_token(&self) -> Option<&str>;
    fn set_sync_token(&mut self, t: Option<String>);
    fn owner_id(&self) -> Option<&str>;
    fn set_owner_id(&mut self, id: Option<String>);
    fn names(&self) -> &HashMap<String, String>;
    fn names_mut(&mut self) -> &mut HashMap<String, String>;
    fn items(&self) -> &HashMap<String, Self::Item>;
    fn items_mut(&mut self) -> &mut HashMap<String, Self::Item>;

    /// Human-readable label used in error messages (e.g. "reminders", "notes").
    fn label() -> &'static str;
    /// Name of the meta table in redb.
    fn meta_table() -> &'static str;
    /// Name of the names table (lists / folders).
    fn names_table() -> &'static str;
    /// Name of the items table (reminders / notes).
    fn items_table() -> &'static str;
    /// If schema_version is missing, treat as empty (Notes) or error (Reminders)?
    fn missing_version_is_empty() -> bool;

    /// Extra key-value pairs to persist in the meta table (JSON-encoded values).
    fn extras(&self) -> Vec<(&'static str, String)> {
        vec![]
    }
    /// Restore extra key-value pairs from the meta table.
    fn load_extras(&mut self, _key: &str, _value: &str) {}

    /// Last-persisted timestamp (ISO 8601, UTC).
    fn updated_at_str(&self) -> Option<&str>;
    fn set_updated_at_str(&mut self, ts: Option<String>);

    /// Access the dirty-tracking state.
    fn ds(&self) -> &DirtyState;
    fn ds_mut(&mut self) -> &mut DirtyState;

    fn is_dirty(&self) -> bool { self.ds().dirty }
    fn mark_dirty(&mut self) { self.ds_mut().dirty = true; }

    /// Extract the display title from an item (for search/find).
    fn item_title(item: &Self::Item) -> &str;

    /// Case-insensitive name (list/folder) lookup by display value.
    fn find_name(&self, name: &str) -> Option<String> {
        let n = name.to_ascii_lowercase();
        self.names()
            .iter()
            .find(|(_, nm)| nm.to_ascii_lowercase() == n)
            .map(|(id, _)| id.clone())
    }

    /// Find an item by exact title (case-insensitive) or record-name prefix/contains.
    fn find_item(&self, partial: &str) -> Option<String> {
        let p = partial.to_ascii_lowercase();
        // Exact title match first
        for (id, item) in self.items() {
            if Self::item_title(item).to_ascii_lowercase() == p {
                return Some(id.clone());
            }
        }
        // Then ID prefix/contains match
        for id in self.items().keys() {
            let check = id.rsplit_once('/').map(|(_, u)| u).unwrap_or(id);
            if check.to_ascii_lowercase().contains(&p) {
                return Some(id.clone());
            }
        }
        None
    }
}

/// On-disk database + companion `.redb.lock` for cross-process exclusion.
#[derive(Debug, Clone)]
pub struct RedbStore<C: StoreCache> {
    db_path: PathBuf,
    lock_path: PathBuf,
    _marker: std::marker::PhantomData<C>,
}

impl<C: StoreCache> RedbStore<C> {
    pub fn new(db_path: impl Into<PathBuf>) -> Self {
        let db_path = db_path.into();
        let lock_path = db_path.with_extension("redb.lock");
        Self { db_path, lock_path, _marker: std::marker::PhantomData }
    }

    pub fn path(&self) -> &Path {
        &self.db_path
    }

    fn wrap(&self, msg: impl std::fmt::Display) -> crate::error::Error {
        crate::error::Error::from_label(C::label(), msg.to_string())
    }

    fn open_db(&self) -> Result<Database> {
        if let Some(p) = self.db_path.parent() {
            fs::create_dir_all(p).map_err(|e| self.wrap(e))?;
        }
        if self.db_path.exists() {
            Database::open(&self.db_path).map_err(|e| self.wrap(format!("redb open: {e}")))
        } else {
            Database::create(&self.db_path).map_err(|e| self.wrap(format!("redb create: {e}")))
        }
    }

    pub fn with_lock<R>(&self, f: impl FnOnce(&Database) -> Result<R>) -> Result<R> {
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(&self.lock_path)
            .map_err(|e| self.wrap(format!("lock file: {e}")))?;
        lock.lock_exclusive().map_err(|e| self.wrap(format!("lock exclusive: {e}")))?;
        let out = (|| {
            let db = self.open_db()?;
            f(&db)
        })();
        lock.unlock().ok();
        out
    }

    pub fn load_cache(&self) -> Result<C> {
        let label = C::label();
        let meta_name = C::meta_table();
        let names_name = C::names_table();
        let items_name = C::items_table();
        let missing_is_empty = C::missing_version_is_empty();

        self.with_lock(|db| {
            let read = db
                .begin_read()
                .map_err(|e| crate::error::Error::from_label(label, format!("redb read txn: {e}")))?;

            let meta_def: TableDefinition<&str, &str> = TableDefinition::new(meta_name);
            let meta = match read.open_table(meta_def) {
                Ok(t) => t,
                Err(e) if is_missing_table(&e) => return Ok(C::default()),
                Err(e) => return Err(crate::error::Error::from_label(label, format!("meta table: {e}"))),
            };

            let schema_cell = meta
                .get(KEY_SCHEMA_VERSION)
                .map_err(|e| crate::error::Error::from_label(label, format!("meta get schema: {e}")))?;
            let ver: String = match schema_cell {
                None if missing_is_empty => return Ok(C::default()),
                None => return Err(crate::error::Error::from_label(
                    label,
                    format!("{label} database is incomplete (missing schema_version); delete the database file and run `icloud {label} sync` again"),
                )),
                Some(v) => v.value().to_string(),
            };
            if ver != EXPECTED_SCHEMA {
                return Err(crate::error::Error::from_label(
                    label,
                    format!("unsupported {label} DB schema {ver:?} (expected {EXPECTED_SCHEMA}); delete the database file and run `icloud {label} sync` again"),
                ));
            }

            let mut cache = C::default();

            if let Some(v) = meta.get(KEY_SYNC_TOKEN).map_err(|e| crate::error::Error::from_label(label, format!("meta: {e}")))? {
                cache.set_sync_token(Some(v.value().to_string()));
            }
            if let Some(v) = meta.get(KEY_OWNER_ID).map_err(|e| crate::error::Error::from_label(label, format!("meta: {e}")))? {
                cache.set_owner_id(Some(v.value().to_string()));
            }
            // Load all remaining meta keys (updated_at, extras)
            for row in meta.iter().map_err(|e| crate::error::Error::from_label(label, format!("meta iter: {e}")))? {
                let (k, v) = row.map_err(|e| crate::error::Error::from_label(label, format!("meta row: {e}")))?;
                let key = k.value();
                let val = v.value();
                match key {
                    KEY_SCHEMA_VERSION | KEY_SYNC_TOKEN | KEY_OWNER_ID => {}
                    KEY_UPDATED_AT => cache.set_updated_at_str(Some(val.to_string())),
                    _ => cache.load_extras(key, val),
                }
            }

            let names_def: TableDefinition<&str, &str> = TableDefinition::new(names_name);
            match read.open_table(names_def) {
                Ok(t) => {
                    for row in t.iter().map_err(|e| crate::error::Error::from_label(label, format!("{names_name} iter: {e}")))? {
                        let (k, v) = row.map_err(|e| crate::error::Error::from_label(label, format!("{names_name} row: {e}")))?;
                        cache.names_mut().insert(k.value().to_string(), v.value().to_string());
                    }
                }
                Err(e) if is_missing_table(&e) => {}
                Err(e) => return Err(crate::error::Error::from_label(label, format!("{names_name}: {e}"))),
            }

            let items_def: TableDefinition<&str, &[u8]> = TableDefinition::new(items_name);
            match read.open_table(items_def) {
                Ok(t) => {
                    for row in t.iter().map_err(|e| crate::error::Error::from_label(label, format!("{items_name} iter: {e}")))? {
                        let (k, v) = row.map_err(|e| crate::error::Error::from_label(label, format!("{items_name} row: {e}")))?;
                        let item: C::Item = serde_json::from_slice(v.value())
                            .map_err(|e| crate::error::Error::from_label(label, format!("{items_name} decode: {e}")))?;
                        cache.items_mut().insert(k.value().to_string(), item);
                    }
                }
                Err(e) if is_missing_table(&e) => {}
                Err(e) => return Err(crate::error::Error::from_label(label, format!("{items_name}: {e}"))),
            }

            Ok(cache)
        })
    }

    /// Persist cache to disk. Uses incremental writes when possible — only
    /// changed/deleted items are touched. Falls back to a full rewrite when
    /// `ds.full_rewrite` is set (force sync / first run).
    pub fn save_cache(&self, cache: &C) -> Result<()> {
        let ds = cache.ds();
        if !ds.dirty {
            return Ok(());
        }

        let label = C::label();
        let meta_name = C::meta_table();
        let names_name = C::names_table();
        let items_name = C::items_table();
        let full = ds.full_rewrite;

        self.with_lock(|db| {
            let write = db
                .begin_write()
                .map_err(|e| crate::error::Error::from_label(label, format!("redb write txn: {e}")))?;

            // Meta — always written in full (tiny, ~5 keys)
            {
                let meta_def: TableDefinition<&str, &str> = TableDefinition::new(meta_name);
                let mut t = write.open_table(meta_def).map_err(|e| crate::error::Error::from_label(label, format!("meta: {e}")))?;
                t.insert(KEY_SCHEMA_VERSION, EXPECTED_SCHEMA).map_err(|e| crate::error::Error::from_label(label, format!("meta schema: {e}")))?;
                match cache.sync_token() {
                    Some(s) => { t.insert(KEY_SYNC_TOKEN, s).map_err(|e| crate::error::Error::from_label(label, format!("meta sync_token: {e}")))?; }
                    None => { t.remove(KEY_SYNC_TOKEN).ok(); }
                }
                match cache.owner_id() {
                    Some(s) => { t.insert(KEY_OWNER_ID, s).map_err(|e| crate::error::Error::from_label(label, format!("meta owner: {e}")))?; }
                    None => { t.remove(KEY_OWNER_ID).ok(); }
                }
                let updated = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S").to_string();
                t.insert(KEY_UPDATED_AT, updated.as_str()).map_err(|e| crate::error::Error::from_label(label, format!("meta updated: {e}")))?;
                for (k, v) in cache.extras() {
                    t.insert(k, v.as_str()).map_err(|e| crate::error::Error::from_label(label, format!("meta extra: {e}")))?;
                }
            }

            // Names (lists / folders)
            {
                let names_def: TableDefinition<&str, &str> = TableDefinition::new(names_name);
                let mut t = write.open_table(names_def).map_err(|e| crate::error::Error::from_label(label, format!("{names_name}: {e}")))?;
                if full {
                    // Full rewrite: truncate then insert all
                    while t.pop_first().map_err(|e| crate::error::Error::from_label(label, format!("{names_name} clear: {e}")))?.is_some() {}
                    for (id, name) in cache.names() {
                        t.insert(id.as_str(), name.as_str()).map_err(|e| crate::error::Error::from_label(label, format!("{names_name} ins: {e}")))?;
                    }
                } else {
                    // Incremental: upsert changed, remove deleted
                    for id in &ds.deleted_names {
                        t.remove(id.as_str()).ok();
                    }
                    for id in &ds.dirty_names {
                        if let Some(name) = cache.names().get(id) {
                            t.insert(id.as_str(), name.as_str()).map_err(|e| crate::error::Error::from_label(label, format!("{names_name} ins: {e}")))?;
                        }
                    }
                }
            }

            // Items (reminders / notes)
            {
                let items_def: TableDefinition<&str, &[u8]> = TableDefinition::new(items_name);
                let mut t = write.open_table(items_def).map_err(|e| crate::error::Error::from_label(label, format!("{items_name}: {e}")))?;
                if full {
                    // Full rewrite: truncate then insert all
                    while t.pop_first().map_err(|e| crate::error::Error::from_label(label, format!("{items_name} clear: {e}")))?.is_some() {}
                    for (id, item) in cache.items() {
                        let bytes = serde_json::to_vec(item).map_err(|e| crate::error::Error::from_label(label, format!("{items_name} enc: {e}")))?;
                        t.insert(id.as_str(), &bytes[..]).map_err(|e| crate::error::Error::from_label(label, format!("{items_name} ins: {e}")))?;
                    }
                } else {
                    // Incremental: upsert changed, remove deleted
                    for id in &ds.deleted_items {
                        t.remove(id.as_str()).ok();
                    }
                    for id in &ds.dirty_items {
                        if let Some(item) = cache.items().get(id) {
                            let bytes = serde_json::to_vec(item).map_err(|e| crate::error::Error::from_label(label, format!("{items_name} enc: {e}")))?;
                            t.insert(id.as_str(), &bytes[..]).map_err(|e| crate::error::Error::from_label(label, format!("{items_name} ins: {e}")))?;
                        }
                    }
                }
            }

            write.commit().map_err(|e| crate::error::Error::from_label(label, format!("redb commit: {e}")))?;
            Ok(())
        })
    }

    pub fn remove_db_file(&self) -> io::Result<()> {
        if self.db_path.exists() {
            fs::remove_file(&self.db_path)
        } else {
            Ok(())
        }
    }
}

/// Returns `true` if the cache was persisted within the last `max_age_secs` seconds.
pub fn is_cache_fresh<C: StoreCache>(cache: &C, max_age_secs: u64) -> bool {
    if max_age_secs == 0 {
        return false;
    }
    let Some(ts) = cache.updated_at_str() else {
        return false;
    };
    let Ok(dt) = chrono::NaiveDateTime::parse_from_str(ts, "%Y-%m-%dT%H:%M:%S") else {
        return false;
    };
    let age = chrono::Utc::now() - dt.and_utc();
    age.num_seconds() >= 0 && age.num_seconds() < max_age_secs as i64
}
