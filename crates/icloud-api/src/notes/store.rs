//! Notes store — thin type alias over the generic RedbStore.

use crate::store::RedbStore;
use super::cache::NotesCache;

pub type NotesStore = RedbStore<NotesCache>;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::notes::cache::NoteData;

    #[test]
    fn roundtrip_empty_and_nonempty() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("notes.redb");
        let store = NotesStore::new(path);
        let mut c = NotesCache::default();
        c.ds.dirty = true;
        c.ds.full_rewrite = true;
        store.save_cache(&c).unwrap();
        let c2 = store.load_cache().unwrap();
        assert!(c2.notes.is_empty());
        assert!(c2.folders.is_empty());

        let mut c = NotesCache::default();
        c.ds.dirty = true;
        c.ds.full_rewrite = true;
        c.folders.insert("F1".into(), "Work".into());
        c.sync_token = Some("tok".into());
        c.notes.insert(
            "N1".into(),
            NoteData {
                title: "Test Note".into(),
                ..Default::default()
            },
        );
        store.save_cache(&c).unwrap();
        let c2 = store.load_cache().unwrap();
        assert_eq!(c2.folders.get("F1").unwrap(), "Work");
        assert_eq!(c2.sync_token.as_deref(), Some("tok"));
        assert_eq!(c2.notes.get("N1").unwrap().title, "Test Note");
    }
}
