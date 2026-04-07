//! Composite iCloud VFS: `/Notes`, `/Reminders`, `/HideMyEmail` + `bashbox::InMemoryFs` elsewhere.

use std::collections::HashSet;
use std::sync::Arc;

use async_trait::async_trait;
use icloud_api::hme::HideMyEmailClient;
use icloud_api::notes::NotesSyncEngine;
use icloud_api::reminders::models::priority_label;
use icloud_api::reminders::SyncEngine;
use icloud_api::title_doc::ts_to_str;
use icloud_api::{with_notes_retry, with_reminders_retry};
use bashbox::fs::types::{CpOptions, DirentEntry, FsError, FsStat, MkdirOptions, RmOptions};
use bashbox::fs::FileSystem;
use bashbox::InMemoryFs;
use tokio::sync::Mutex;

use crate::frontmatter::{
    parse_reminder_write_input, render_note, render_reminder, strip_frontmatter, NoteFrontmatter,
    ReminderFrontmatter,
};
use crate::pathmap::{classify, is_icloud_prefix, normalize_vpath, VfsTarget};
use crate::sanitize::{disambiguate_filename, filename_to_title, title_to_filename_stem};

const MAX_ICLOUD_BYTES: usize = 1024 * 1024;

#[derive(Debug, Clone)]
struct DisambiguatedEntry {
    id: String,
    filename: String,
}

fn map_api_err(path: &str, op: &str, e: icloud_api::Error) -> FsError {
    let msg = e.to_string();
    if msg.contains("401") || msg.contains("403") || msg.contains("session") {
        return FsError::Other {
            message: format!("EIO: Session expired — run `icloud login` ({msg})"),
        };
    }
    FsError::Other {
        message: format!("EIO: {msg} ({op} '{path}')"),
    }
}

fn sorted_names(values: impl Iterator<Item = String>) -> Vec<String> {
    let mut names: Vec<String> = values.collect();
    names.sort();
    names
}

fn build_disambiguated_entries<I>(items: I) -> Vec<DisambiguatedEntry>
where
    I: IntoIterator<Item = (String, String)>,
{
    let mut items: Vec<(String, String)> = items.into_iter().collect();
    items.sort_by(|(left_id, left_title), (right_id, right_title)| {
        left_title
            .to_ascii_lowercase()
            .cmp(&right_title.to_ascii_lowercase())
            .then_with(|| left_title.cmp(right_title))
            .then_with(|| left_id.cmp(right_id))
    });

    let mut existing = HashSet::with_capacity(items.len());
    items
        .into_iter()
        .map(|(id, title)| {
            let stem = title_to_filename_stem(&title);
            let filename = disambiguate_filename(&stem, &existing);
            existing.insert(filename.to_ascii_lowercase());
            DisambiguatedEntry { id, filename }
        })
        .collect()
}

fn note_entries(eng: &NotesSyncEngine, folder_name: &str) -> Vec<DisambiguatedEntry> {
    let Some(folder_id) = eng.cache.find_folder_by_name(folder_name) else {
        return Vec::new();
    };

    build_disambiguated_entries(eng.cache.notes.iter().filter_map(|(id, note)| {
        (!note.deleted && note.folder_ref.as_deref() == Some(folder_id.as_str()))
            .then(|| (id.clone(), note.title.clone()))
    }))
}

fn reminder_entries(eng: &SyncEngine, list_name: &str) -> Vec<DisambiguatedEntry> {
    let Some(list_id) = eng.cache.find_list_by_name(list_name) else {
        return Vec::new();
    };

    build_disambiguated_entries(eng.cache.reminders.iter().filter_map(|(id, reminder)| {
        (reminder.list_ref.as_deref() == Some(list_id.as_str()))
            .then(|| (id.clone(), reminder.title.clone()))
    }))
}

fn resolve_note_id(eng: &NotesSyncEngine, folder: &str, filename: &str) -> Option<String> {
    let title_guess = filename_to_title(filename);
    note_entries(eng, folder)
        .into_iter()
        .find(|entry| entry.filename == filename)
        .map(|entry| entry.id)
        .or_else(|| eng.cache.find_note(&title_guess))
}

fn resolve_reminder_id(eng: &SyncEngine, list: &str, filename: &str) -> Option<String> {
    let title_guess = filename_to_title(filename);
    reminder_entries(eng, list)
        .into_iter()
        .find(|entry| entry.filename == filename)
        .map(|entry| entry.id)
        .or_else(|| eng.cache.find_reminder(&title_guess))
}

fn is_icloud_path(path: &str) -> bool {
    is_icloud_prefix(&normalize_vpath(path))
}

/// Virtual filesystem: iCloud-backed subtrees + [`InMemoryFs`] for `/bin`, `/tmp`, etc.
pub struct ICloudFs {
    inner: Arc<InMemoryFs>,
    notes: Arc<Mutex<NotesSyncEngine>>,
    reminders: Arc<Mutex<SyncEngine>>,
    hme: Arc<HideMyEmailClient>,
    aliases_cache: Arc<Mutex<Option<String>>>,
}

impl ICloudFs {
    pub fn new(
        inner: Arc<InMemoryFs>,
        notes: Arc<Mutex<NotesSyncEngine>>,
        reminders: Arc<Mutex<SyncEngine>>,
        hme: Arc<HideMyEmailClient>,
    ) -> Self {
        Self {
            inner,
            notes,
            reminders,
            hme,
            aliases_cache: Arc::new(Mutex::new(None)),
        }
    }

    async fn read_note_file(&self, folder_name: &str, filename: &str) -> Result<String, FsError> {
        let mut eng = self.notes.lock().await;
        eng.cache
            .find_folder_by_name(folder_name)
            .ok_or_else(|| FsError::NotFound {
                path: folder_name.to_string(),
                operation: "open".to_string(),
            })?;
        let note_id = resolve_note_id(&eng, folder_name, filename);
        let note_id = note_id.ok_or_else(|| FsError::NotFound {
            path: filename.to_string(),
            operation: "open".to_string(),
        })?;
        let nd = eng
            .cache
            .notes
            .get(&note_id)
            .cloned()
            .ok_or_else(|| FsError::NotFound {
                path: filename.to_string(),
                operation: "open".to_string(),
            })?;
        let body = eng
            .fetch_body(&note_id)
            .await
            .map_err(|e| map_api_err(&format!("/Notes/{folder_name}/{filename}"), "read", e))?;
        let modified = nd.modified_ts.and_then(ts_to_str).unwrap_or_default();
        let fm = NoteFrontmatter {
            id: note_id.clone(),
            folder: folder_name.to_string(),
            modified,
        };
        Ok(render_note(&fm, &body))
    }

    async fn read_reminder_file(&self, list_name: &str, filename: &str) -> Result<String, FsError> {
        let eng = self.reminders.lock().await;
        let _list_id = eng
            .cache
            .find_list_by_name(list_name)
            .ok_or_else(|| FsError::NotFound {
                path: list_name.to_string(),
                operation: "open".to_string(),
            })?;
        let rid =
            resolve_reminder_id(&eng, list_name, filename).ok_or_else(|| FsError::NotFound {
                path: filename.to_string(),
                operation: "open".to_string(),
            })?;
        let rd = eng
            .cache
            .reminders
            .get(&rid)
            .cloned()
            .ok_or_else(|| FsError::NotFound {
                path: filename.to_string(),
                operation: "open".to_string(),
            })?;
        let pri = priority_label(rd.priority);
        let pri_s = if pri.is_empty() {
            "none".to_string()
        } else {
            pri.to_string()
        };
        let fm = ReminderFrontmatter {
            id: rid.clone(),
            list: list_name.to_string(),
            completed: rd.completed,
            due: rd.due.clone(),
            priority: pri_s,
            notes: rd.notes.clone().unwrap_or_default(),
        };
        Ok(render_reminder(&fm, &rd.title))
    }

    /// All iCloud-backed paths for virtual glob matching (see `bashbox::SyncFsAdapter::glob`).
    async fn push_ic_paths(&self, paths: &mut Vec<String>) {
        paths.push("/Notes".into());
        paths.push("/Reminders".into());
        paths.push("/HideMyEmail".into());
        paths.push("/HideMyEmail/aliases.json".into());

        {
            let n = self.notes.lock().await;
            for folder_name in sorted_names(n.cache.folders.values().cloned()) {
                paths.push(format!("/Notes/{folder_name}"));
                for entry in note_entries(&n, &folder_name) {
                    paths.push(format!("/Notes/{folder_name}/{}", entry.filename));
                }
            }
        }

        {
            let r = self.reminders.lock().await;
            for list_name in sorted_names(r.cache.lists.values().cloned()) {
                paths.push(format!("/Reminders/{list_name}"));
                for entry in reminder_entries(&r, &list_name) {
                    paths.push(format!("/Reminders/{list_name}/{}", entry.filename));
                }
            }
        }
    }
}

#[async_trait]
impl FileSystem for ICloudFs {
    async fn read_file(&self, path: &str) -> Result<String, FsError> {
        if !is_icloud_path(path) {
            return self.inner.read_file(path).await;
        }
        let n = normalize_vpath(path);
        match classify(&n).map_err(|_| FsError::InvalidArgument {
            path: path.to_string(),
            operation: "open".to_string(),
        })? {
            VfsTarget::HideMyEmailAliases => {
                let mut cache = self.aliases_cache.lock().await;
                if cache.is_none() {
                    let v = self
                        .hme
                        .list_aliases()
                        .await
                        .map_err(|e| map_api_err(path, "read", e))?;
                    *cache =
                        Some(serde_json::to_string_pretty(&v).unwrap_or_else(|_| "{}".to_string()));
                }
                Ok(cache.as_ref().unwrap().clone())
            }
            VfsTarget::NotesFile {
                folder_name,
                filename,
            } => self.read_note_file(&folder_name, &filename).await,
            VfsTarget::RemindersFile {
                list_name,
                filename,
            } => self.read_reminder_file(&list_name, &filename).await,
            VfsTarget::NotesFolder { .. }
            | VfsTarget::RemindersList { .. }
            | VfsTarget::Root
            | VfsTarget::NotesRoot
            | VfsTarget::RemindersRoot
            | VfsTarget::HideMyEmailRoot => Err(FsError::IsDirectory {
                path: path.to_string(),
                operation: "read".to_string(),
            }),
            VfsTarget::Passthrough => Err(FsError::NotFound {
                path: path.to_string(),
                operation: "open".to_string(),
            }),
        }
    }

    async fn read_file_buffer(&self, path: &str) -> Result<Vec<u8>, FsError> {
        let s = self.read_file(path).await?;
        Ok(s.into_bytes())
    }

    async fn write_file(&self, path: &str, content: &[u8]) -> Result<(), FsError> {
        if !is_icloud_path(path) {
            return self.inner.write_file(path, content).await;
        }
        if content.len() > MAX_ICLOUD_BYTES {
            return Err(FsError::InvalidArgument {
                path: path.to_string(),
                operation: "write".to_string(),
            });
        }
        let n = normalize_vpath(path);
        let text = String::from_utf8(content.to_vec())
            .unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned());
        match classify(&n).map_err(|_| FsError::InvalidArgument {
            path: path.to_string(),
            operation: "write".to_string(),
        })? {
            VfsTarget::NotesFile {
                folder_name,
                filename,
            } => {
                let (body, _) = strip_frontmatter(&text);
                let title_from_file = filename_to_title(&filename);
                let md = if body
                    .lines()
                    .next()
                    .map(|l| l.starts_with('#'))
                    .unwrap_or(false)
                {
                    body.into_owned()
                } else {
                    format!("# {title_from_file}\n\n{body}")
                };
                let mut eng = self.notes.lock().await;
                eng.cache
                    .find_folder_by_name(&folder_name)
                    .ok_or_else(|| FsError::NotFound {
                        path: folder_name.clone(),
                        operation: "write".to_string(),
                    })?;
                let existing_id = resolve_note_id(&eng, &folder_name, &filename);
                if let Some(id) = existing_id {
                    let id = id.clone();
                    let md = md.clone();
                    with_notes_retry(&mut eng, |e| {
                        let id = id.clone();
                        let md = md.clone();
                        Box::pin(async move { e.update_note(&id, &md).await })
                    })
                    .await
                    .map_err(|e| map_api_err(path, "write", e))?;
                } else {
                    let md = md.clone();
                    let folder = folder_name.clone();
                    with_notes_retry(&mut eng, |e| {
                        let md = md.clone();
                        let folder = folder.clone();
                        Box::pin(async move { e.create_note(&md, &folder).await })
                    })
                    .await
                    .map_err(|e| map_api_err(path, "write", e))?;
                }
                Ok(())
            }
            VfsTarget::RemindersFile {
                list_name,
                filename,
            } => {
                let (fields, body) =
                    parse_reminder_write_input(&text).map_err(|e| FsError::Other { message: e })?;
                let title = {
                    let t = body.trim();
                    if t.is_empty() {
                        filename_to_title(&filename)
                    } else {
                        t.to_string()
                    }
                };
                let mut eng = self.reminders.lock().await;
                eng.cache
                    .find_list_by_name(&list_name)
                    .ok_or_else(|| FsError::NotFound {
                        path: list_name.clone(),
                        operation: "write".to_string(),
                    })?;
                let existing = resolve_reminder_id(&eng, &list_name, &filename);
                if let Some(rid) = existing {
                    if let Some(c) = fields.completed {
                        let rid_c = rid.clone();
                        if c {
                            with_reminders_retry(&mut eng, |e| {
                                let rid_c = rid_c.clone();
                                Box::pin(async move { e.complete_reminder(&rid_c).await })
                            })
                            .await
                            .map_err(|e| map_api_err(path, "write", e))?;
                        } else {
                            with_reminders_retry(&mut eng, |e| {
                                let rid_c = rid_c.clone();
                                Box::pin(async move { e.uncomplete_reminder(&rid_c).await })
                            })
                            .await
                            .map_err(|e| map_api_err(path, "write", e))?;
                        }
                    }
                    let due_flat = fields.due.clone().flatten();
                    let need_edit = fields.due.is_some()
                        || fields.clear_due
                        || fields.notes.is_some()
                        || fields.priority.is_some();
                    if need_edit {
                        let rid = rid.clone();
                        let title = title.clone();
                        let clear_due = fields.clear_due;
                        let notes = fields.notes.clone();
                        let pri = fields.priority.clone();
                        with_reminders_retry(&mut eng, |e| {
                            let rid = rid.clone();
                            let title = title.clone();
                            let due_flat = due_flat.clone();
                            let notes = notes.clone();
                            let pri = pri.clone();
                            Box::pin(async move {
                                e.edit_reminder(
                                    &rid,
                                    Some(&title),
                                    due_flat.as_deref(),
                                    clear_due,
                                    notes.as_deref(),
                                    pri.as_deref(),
                                )
                                .await
                            })
                        })
                        .await
                        .map_err(|e| map_api_err(path, "write", e))?;
                    }
                } else {
                    let title = title.clone();
                    let list = list_name.clone();
                    let due = fields.due.clone().flatten();
                    let pri = fields.priority.clone();
                    let notes = fields.notes.clone();
                    with_reminders_retry(&mut eng, |e| {
                        let title = title.clone();
                        let list = list.clone();
                        let due = due.clone();
                        let pri = pri.clone();
                        let notes = notes.clone();
                        Box::pin(async move {
                            e.add_reminder(
                                &title,
                                &list,
                                due.as_deref(),
                                pri.as_deref(),
                                notes.as_deref(),
                                None,
                            )
                            .await
                        })
                    })
                    .await
                    .map_err(|e| map_api_err(path, "write", e))?;
                }
                Ok(())
            }
            VfsTarget::Root => Err(FsError::PermissionDenied {
                path: path.to_string(),
                operation: "write".to_string(),
            }),
            VfsTarget::HideMyEmailRoot | VfsTarget::HideMyEmailAliases => Err(FsError::ReadOnly {
                operation: "write".to_string(),
            }),
            _ => Err(FsError::IsDirectory {
                path: path.to_string(),
                operation: "write".to_string(),
            }),
        }
    }

    async fn append_file(&self, path: &str, content: &[u8]) -> Result<(), FsError> {
        if !is_icloud_path(path) {
            return self.inner.append_file(path, content).await;
        }
        let cur = self.read_file(path).await?;
        let mut combined = cur.into_bytes();
        combined.extend_from_slice(content);
        self.write_file(path, &combined).await
    }

    async fn exists(&self, path: &str) -> bool {
        if !is_icloud_path(path) {
            return self.inner.exists(path).await;
        }
        self.stat(path).await.is_ok()
    }

    async fn stat(&self, path: &str) -> Result<FsStat, FsError> {
        if !is_icloud_path(path) {
            return self.inner.stat(path).await;
        }
        let n = normalize_vpath(path);
        let target = classify(&n).map_err(|_| FsError::NotFound {
            path: path.to_string(),
            operation: "stat".to_string(),
        })?;
        let is_dir = matches!(
            target,
            VfsTarget::Root
                | VfsTarget::NotesRoot
                | VfsTarget::NotesFolder { .. }
                | VfsTarget::RemindersRoot
                | VfsTarget::RemindersList { .. }
                | VfsTarget::HideMyEmailRoot
        );
        // Return size 0 for all iCloud targets — avoids fetching every note/reminder
        // body over the network just to report content length (catastrophic for 10K+ items).
        if matches!(target, VfsTarget::Passthrough) {
            unreachable!();
        }
        Ok(FsStat {
            is_file: !is_dir,
            is_directory: is_dir,
            is_symlink: false,
            mode: if is_dir { 0o755 } else { 0o644 },
            size: 0,
            mtime: std::time::SystemTime::UNIX_EPOCH,
        })
    }

    async fn lstat(&self, path: &str) -> Result<FsStat, FsError> {
        self.stat(path).await
    }

    async fn mkdir(&self, path: &str, options: &MkdirOptions) -> Result<(), FsError> {
        if !is_icloud_path(path) {
            return self.inner.mkdir(path, options).await;
        }
        let n = normalize_vpath(path);
        match classify(&n) {
            Ok(VfsTarget::NotesFolder { folder_name }) => {
                let mut eng = self.notes.lock().await;
                let name = folder_name.clone();
                with_notes_retry(&mut eng, |e| {
                    let name = name.clone();
                    Box::pin(async move { e.create_folder(&name).await })
                })
                .await
                .map_err(|e| map_api_err(path, "mkdir", e))?;
                Ok(())
            }
            Ok(VfsTarget::RemindersList { list_name }) => {
                let mut eng = self.reminders.lock().await;
                let name = list_name.clone();
                with_reminders_retry(&mut eng, |e| {
                    let name = name.clone();
                    Box::pin(async move { e.create_list(&name).await })
                })
                .await
                .map_err(|e| map_api_err(path, "mkdir", e))?;
                Ok(())
            }
            Ok(VfsTarget::Root) => Err(FsError::PermissionDenied {
                path: path.to_string(),
                operation: "mkdir".to_string(),
            }),
            Ok(VfsTarget::HideMyEmailRoot) | Ok(VfsTarget::HideMyEmailAliases) => {
                Err(FsError::ReadOnly {
                    operation: "mkdir".to_string(),
                })
            }
            _ => Err(FsError::InvalidArgument {
                path: path.to_string(),
                operation: "mkdir".to_string(),
            }),
        }
    }

    async fn readdir(&self, path: &str) -> Result<Vec<String>, FsError> {
        let v = self.readdir_with_file_types(path).await?;
        Ok(v.into_iter().map(|e| e.name).collect())
    }

    async fn readdir_with_file_types(&self, path: &str) -> Result<Vec<DirentEntry>, FsError> {
        if !is_icloud_path(path) {
            return self.inner.readdir_with_file_types(path).await;
        }
        let n = normalize_vpath(path);
        match classify(&n).map_err(|_| FsError::InvalidArgument {
            path: path.to_string(),
            operation: "scandir".to_string(),
        })? {
            VfsTarget::Root => Ok(vec![
                dent("Notes", true),
                dent("Reminders", true),
                dent("HideMyEmail", true),
                dent("tmp", true),
            ]),
            VfsTarget::NotesRoot => {
                let eng = self.notes.lock().await;
                Ok(sorted_names(eng.cache.folders.values().cloned())
                    .into_iter()
                    .map(|name| dent(&name, true))
                    .collect())
            }
            VfsTarget::NotesFolder { folder_name } => {
                let eng = self.notes.lock().await;
                Ok(note_entries(&eng, &folder_name)
                    .into_iter()
                    .map(|entry| dent(&entry.filename, false))
                    .collect())
            }
            VfsTarget::RemindersRoot => {
                let eng = self.reminders.lock().await;
                Ok(sorted_names(eng.cache.lists.values().cloned())
                    .into_iter()
                    .map(|name| dent(&name, true))
                    .collect())
            }
            VfsTarget::RemindersList { list_name } => {
                let eng = self.reminders.lock().await;
                Ok(reminder_entries(&eng, &list_name)
                    .into_iter()
                    .map(|entry| dent(&entry.filename, false))
                    .collect())
            }
            VfsTarget::HideMyEmailRoot => Ok(vec![dent("aliases.json", false)]),
            _ => Err(FsError::NotDirectory {
                path: path.to_string(),
                operation: "scandir".to_string(),
            }),
        }
    }

    async fn rm(&self, path: &str, options: &RmOptions) -> Result<(), FsError> {
        if !is_icloud_path(path) {
            return self.inner.rm(path, options).await;
        }
        let n = normalize_vpath(path);
        match classify(&n).map_err(|_| FsError::InvalidArgument {
            path: path.to_string(),
            operation: "rm".to_string(),
        })? {
            VfsTarget::NotesFile {
                folder_name,
                filename,
            } => {
                let mut eng = self.notes.lock().await;
                let id = resolve_note_id(&eng, &folder_name, &filename).ok_or_else(|| {
                    FsError::NotFound {
                        path: path.to_string(),
                        operation: "rm".to_string(),
                    }
                })?;
                let id = id.clone();
                with_notes_retry(&mut eng, |e| {
                    let id = id.clone();
                    Box::pin(async move { e.delete_note(&id).await })
                })
                .await
                .map_err(|e| map_api_err(path, "rm", e))?;
                Ok(())
            }
            VfsTarget::RemindersFile {
                list_name,
                filename,
            } => {
                let mut eng = self.reminders.lock().await;
                let id = resolve_reminder_id(&eng, &list_name, &filename).ok_or_else(|| {
                    FsError::NotFound {
                        path: path.to_string(),
                        operation: "rm".to_string(),
                    }
                })?;
                let id = id.clone();
                with_reminders_retry(&mut eng, |e| {
                    let id = id.clone();
                    Box::pin(async move { e.delete_reminder(&id).await })
                })
                .await
                .map_err(|e| map_api_err(path, "rm", e))?;
                Ok(())
            }
            VfsTarget::NotesFolder { folder_name } => {
                let mut eng = self.notes.lock().await;
                let name = folder_name.clone();
                with_notes_retry(&mut eng, |e| {
                    let name = name.clone();
                    Box::pin(async move { e.delete_folder(&name).await })
                })
                .await
                .map_err(|e| map_api_err(path, "rm", e))?;
                Ok(())
            }
            VfsTarget::RemindersList { list_name } => {
                let mut eng = self.reminders.lock().await;
                let name = list_name.clone();
                with_reminders_retry(&mut eng, |e| {
                    let name = name.clone();
                    Box::pin(async move { e.delete_list(&name).await })
                })
                .await
                .map_err(|e| map_api_err(path, "rm", e))?;
                Ok(())
            }
            _ => Err(FsError::IsDirectory {
                path: path.to_string(),
                operation: "rm".to_string(),
            }),
        }
    }

    async fn cp(&self, src: &str, dest: &str, options: &CpOptions) -> Result<(), FsError> {
        let s_ic = is_icloud_path(src);
        let d_ic = is_icloud_path(dest);
        if s_ic && d_ic {
            let sn = normalize_vpath(src);
            let dn = normalize_vpath(dest);
            match (classify(&sn), classify(&dn)) {
                (
                    Ok(VfsTarget::NotesFile {
                        folder_name: _f1, ..
                    }),
                    Ok(VfsTarget::NotesFile {
                        folder_name: f2,
                        filename: df,
                    }),
                ) => {
                    let body = self.read_file(src).await?;
                    let (text, _) = strip_frontmatter(&body);
                    let mut eng = self.notes.lock().await;
                    let md = if text
                        .lines()
                        .next()
                        .map(|l| l.starts_with('#'))
                        .unwrap_or(false)
                    {
                        text.into_owned()
                    } else {
                        let title = filename_to_title(&df);
                        format!("# {title}\n\n{text}")
                    };
                    let md = md.clone();
                    let f = f2.clone();
                    with_notes_retry(&mut eng, |e| {
                        let md = md.clone();
                        let f = f.clone();
                        Box::pin(async move { e.create_note(&md, &f).await })
                    })
                    .await
                    .map_err(|e| map_api_err(dest, "cp", e))?;
                    Ok(())
                }
                (Ok(VfsTarget::NotesFile { .. }), Ok(VfsTarget::RemindersFile { .. }))
                | (Ok(VfsTarget::RemindersFile { .. }), Ok(VfsTarget::NotesFile { .. })) => {
                    Err(FsError::Other {
                        message: "EXDEV: invalid cross-device link".into(),
                    })
                }
                _ => Err(FsError::InvalidArgument {
                    path: src.to_string(),
                    operation: "cp".to_string(),
                }),
            }
        } else if !s_ic && !d_ic {
            self.inner.cp(src, dest, options).await
        } else if s_ic && !d_ic {
            let b = self.read_file(src).await?;
            self.inner.write_file(dest, b.as_bytes()).await
        } else {
            let b = self.inner.read_file_buffer(src).await?;
            self.write_file(dest, &b).await
        }
    }

    async fn mv(&self, src: &str, dest: &str) -> Result<(), FsError> {
        let s_ic = is_icloud_path(src);
        let d_ic = is_icloud_path(dest);
        if s_ic && d_ic {
            let sn = normalize_vpath(src);
            let dn = normalize_vpath(dest);
            match (classify(&sn), classify(&dn)) {
                (
                    Ok(VfsTarget::NotesFile {
                        folder_name: f1,
                        filename: sf,
                    }),
                    Ok(VfsTarget::NotesFile {
                        folder_name: f2,
                        filename: nf,
                    }),
                ) => {
                    let body = self.read_file(src).await?;
                    let (rest, _) = strip_frontmatter(&body);
                    let new_title = filename_to_title(&nf);
                    let md = if rest.trim_start().starts_with('#') {
                        let mut lines = rest.lines();
                        let _ = lines.next();
                        format!("# {new_title}\n{}", lines.collect::<Vec<_>>().join("\n"))
                    } else {
                        format!("# {new_title}\n\n{rest}")
                    };
                    let mut eng = self.notes.lock().await;
                    let id = resolve_note_id(&eng, &f1, &sf).ok_or_else(|| FsError::NotFound {
                        path: src.to_string(),
                        operation: "mv".to_string(),
                    })?;
                    if f1 != f2 {
                        let id_m = id.clone();
                        let f = f2.clone();
                        with_notes_retry(&mut eng, |e| {
                            let id_m = id_m.clone();
                            let f = f.clone();
                            Box::pin(async move { e.move_note(&id_m, &f).await })
                        })
                        .await
                        .map_err(|e| map_api_err(dest, "mv", e))?;
                        let id_u = id.clone();
                        let md_u = md.clone();
                        with_notes_retry(&mut eng, |e| {
                            let id_u = id_u.clone();
                            let md_u = md_u.clone();
                            Box::pin(async move { e.update_note(&id_u, &md_u).await })
                        })
                        .await
                        .map_err(|e| map_api_err(dest, "mv", e))?;
                    } else {
                        let id_u = id.clone();
                        let md_u = md.clone();
                        with_notes_retry(&mut eng, |e| {
                            let id_u = id_u.clone();
                            let md_u = md_u.clone();
                            Box::pin(async move { e.update_note(&id_u, &md_u).await })
                        })
                        .await
                        .map_err(|e| map_api_err(dest, "mv", e))?;
                    }
                    Ok(())
                }
                (
                    Ok(VfsTarget::RemindersFile {
                        list_name: l1,
                        filename: sf,
                    }),
                    Ok(VfsTarget::RemindersFile {
                        list_name: l2,
                        filename: df,
                    }),
                ) => {
                    if l1 != l2 {
                        return Err(FsError::Other {
                            message: "EXDEV: cannot move reminders between lists".into(),
                        });
                    }
                    let new_title = filename_to_title(&df);
                    let mut eng = self.reminders.lock().await;
                    let rid =
                        resolve_reminder_id(&eng, &l1, &sf).ok_or_else(|| FsError::NotFound {
                            path: src.to_string(),
                            operation: "mv".to_string(),
                        })?;
                    let rid_m = rid.clone();
                    let t = new_title.clone();
                    with_reminders_retry(&mut eng, |e| {
                        let rid_m = rid_m.clone();
                        let t = t.clone();
                        Box::pin(async move {
                            e.edit_reminder(&rid_m, Some(&t), None, false, None, None)
                                .await
                        })
                    })
                    .await
                    .map_err(|e| map_api_err(dest, "mv", e))?;
                    Ok(())
                }
                _ => Err(FsError::Other {
                    message: "EXDEV: invalid cross-device link".into(),
                }),
            }
        } else if !s_ic && !d_ic {
            self.inner.mv(src, dest).await
        } else {
            Err(FsError::Other {
                message: "EXDEV: invalid cross-device link".into(),
            })
        }
    }

    async fn chmod(&self, path: &str, mode: u32) -> Result<(), FsError> {
        if !is_icloud_path(path) {
            return self.inner.chmod(path, mode).await;
        }
        Ok(())
    }

    async fn symlink(&self, target: &str, link_path: &str) -> Result<(), FsError> {
        if is_icloud_path(link_path) {
            return Err(FsError::PermissionDenied {
                path: link_path.to_string(),
                operation: "symlink".to_string(),
            });
        }
        self.inner.symlink(target, link_path).await
    }

    async fn link(&self, existing_path: &str, new_path: &str) -> Result<(), FsError> {
        if is_icloud_path(existing_path) || is_icloud_path(new_path) {
            return Err(FsError::PermissionDenied {
                path: existing_path.to_string(),
                operation: "link".to_string(),
            });
        }
        self.inner.link(existing_path, new_path).await
    }

    async fn readlink(&self, path: &str) -> Result<String, FsError> {
        if is_icloud_path(path) {
            return Err(FsError::InvalidArgument {
                path: path.to_string(),
                operation: "readlink".to_string(),
            });
        }
        self.inner.readlink(path).await
    }

    async fn realpath(&self, path: &str) -> Result<String, FsError> {
        if !is_icloud_path(path) {
            return self.inner.realpath(path).await;
        }
        Ok(normalize_vpath(path))
    }

    async fn utimes(&self, path: &str, mtime: std::time::SystemTime) -> Result<(), FsError> {
        if !is_icloud_path(path) {
            return self.inner.utimes(path, mtime).await;
        }
        Ok(())
    }

    fn resolve_path(&self, base: &str, path: &str) -> String {
        if path.starts_with('/') {
            normalize_vpath(path)
        } else if base == "/" {
            normalize_vpath(&format!("/{path}"))
        } else {
            normalize_vpath(&format!("{base}/{path}"))
        }
    }

    fn get_all_paths(&self) -> Vec<String> {
        let mut paths = self.inner.get_all_paths();
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.block_on(self.push_ic_paths(&mut paths));
        } else {
            paths.push("/Notes".into());
            paths.push("/Reminders".into());
            paths.push("/HideMyEmail".into());
            paths.push("/HideMyEmail/aliases.json".into());
        }
        paths.sort();
        paths.dedup();
        paths
    }
}

fn dent(name: &str, is_dir: bool) -> DirentEntry {
    DirentEntry {
        name: name.to_string(),
        is_file: !is_dir,
        is_directory: is_dir,
        is_symlink: false,
    }
}
