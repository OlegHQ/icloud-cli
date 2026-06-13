use std::{env, process::Command};

use icloud_api::notes::{NoteData, NotesCache, NotesStore};

const WORKER_TEST: &str = "store_process_lock_worker";
const DB_ENV: &str = "ICLOUD_STORE_PROCESS_LOCK_DB";
const WORKER_ENV: &str = "ICLOUD_STORE_PROCESS_LOCK_WORKER";
const WORKER_COUNT: usize = 8;

#[test]
fn notes_store_serializes_multi_process_writes() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("notes.redb");
    let store = NotesStore::new(&db_path);

    let mut cache = NotesCache::default();
    cache.folders.insert("folder-id".into(), "Notes".into());
    cache.ds.dirty = true;
    cache.ds.full_rewrite = true;
    store.save_cache(&mut cache).expect("seed notes store");

    let current_exe = env::current_exe().expect("current test binary");
    let mut children = Vec::with_capacity(WORKER_COUNT);
    for worker in 0..WORKER_COUNT {
        children.push(
            Command::new(&current_exe)
                .arg("--exact")
                .arg(WORKER_TEST)
                .arg("--ignored")
                .env(DB_ENV, &db_path)
                .env(WORKER_ENV, worker.to_string())
                .spawn()
                .expect("spawn store worker"),
        );
    }

    for mut child in children {
        let status = child.wait().expect("wait for store worker");
        assert!(status.success(), "worker failed: {status}");
    }

    let cache = store.load_cache().expect("reload notes store");
    for worker in 0..WORKER_COUNT {
        let id = format!("worker-note-{worker}");
        let note = cache.notes.get(&id).unwrap_or_else(|| {
            panic!(
                "missing {id}; present notes: {:?}",
                cache.notes.keys().collect::<Vec<_>>()
            )
        });
        assert_eq!(note.title, format!("worker title {worker}"));
        assert_eq!(note.folder_ref.as_deref(), Some("folder-id"));
    }
}

#[test]
#[ignore = "worker entrypoint invoked by notes_store_serializes_multi_process_writes"]
fn store_process_lock_worker() {
    let Ok(db_path) = env::var(DB_ENV) else {
        return;
    };
    let worker: usize = env::var(WORKER_ENV)
        .expect("worker id")
        .parse()
        .expect("numeric worker id");

    let store = NotesStore::new(db_path);
    let mut cache = store.load_cache().expect("worker load notes store");
    let id = format!("worker-note-{worker}");
    cache.notes.insert(
        id.clone(),
        NoteData {
            title: format!("worker title {worker}"),
            folder_ref: Some("folder-id".into()),
            ..Default::default()
        },
    );
    cache.ds.item_changed(id);
    store
        .save_cache(&mut cache)
        .expect("worker save notes store");
}
