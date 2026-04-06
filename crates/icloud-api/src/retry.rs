//! Retry CloudKit mutations after incremental sync when the server reports a stale record.

use std::future::Future;
use std::pin::Pin;

use crate::error::{Error, Result};

pub fn is_cloudkit_retryable(err: &Error) -> bool {
    let s = err.to_string().to_lowercase();
    s.contains("changetag")
        || s.contains("conflict")
        || s.contains("server record changed")
        || s.contains("stale")
        || s.contains("record changed")
        || s.contains("oplock")
}

pub async fn with_notes_retry<F, T>(
    engine: &mut crate::notes::NotesSyncEngine,
    mut op: F,
) -> Result<T>
where
    F: for<'a> FnMut(
        &'a mut crate::notes::NotesSyncEngine,
    ) -> Pin<Box<dyn Future<Output = Result<T>> + Send + 'a>>,
{
    for attempt in 0..3 {
        match op(engine).await {
            Ok(v) => return Ok(v),
            Err(e) if attempt < 2 && is_cloudkit_retryable(&e) => {
                engine.sync(false).await?;
            }
            Err(e) => return Err(e),
        }
    }
    unreachable!()
}

pub async fn with_reminders_retry<F, T>(
    engine: &mut crate::reminders::SyncEngine,
    mut op: F,
) -> Result<T>
where
    F: for<'a> FnMut(
        &'a mut crate::reminders::SyncEngine,
    ) -> Pin<Box<dyn Future<Output = Result<T>> + Send + 'a>>,
{
    for attempt in 0..3 {
        match op(engine).await {
            Ok(v) => return Ok(v),
            Err(e) if attempt < 2 && is_cloudkit_retryable(&e) => {
                engine.sync(false).await?;
            }
            Err(e) => return Err(e),
        }
    }
    unreachable!()
}
