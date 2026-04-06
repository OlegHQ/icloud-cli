//! Reminders store — thin type alias over the generic RedbStore.

use crate::store::RedbStore;
use super::cache::RemindersCache;

pub type RemindersStore = RedbStore<RemindersCache>;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reminders::cache::ReminderData;

    #[test]
    fn roundtrip_empty_and_nonempty() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.redb");
        let store = RemindersStore::new(path);
        let mut c = RemindersCache::default();
        c.ds.dirty = true;
        c.ds.full_rewrite = true;
        store.save_cache(&c).unwrap();
        let c2 = store.load_cache().unwrap();
        assert!(c2.reminders.is_empty());
        assert!(c2.lists.is_empty());

        let mut c = RemindersCache::default();
        c.ds.dirty = true;
        c.ds.full_rewrite = true;
        c.lists.insert("L1".into(), "Groceries".into());
        c.sync_token = Some("tok".into());
        c.reminders.insert(
            "R1".into(),
            ReminderData {
                title: "buy milk".into(),
                ..Default::default()
            },
        );
        store.save_cache(&c).unwrap();
        let c2 = store.load_cache().unwrap();
        assert_eq!(c2.lists.get("L1").unwrap(), "Groceries");
        assert_eq!(c2.sync_token.as_deref(), Some("tok"));
        assert_eq!(c2.reminders.get("R1").unwrap().title, "buy milk");
    }
}
