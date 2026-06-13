//! Retry CloudKit mutations after incremental sync when the server reports a stale record.

use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use crate::error::{Error, Result};

fn retry_delay(attempt: usize) -> Duration {
    Duration::from_millis(250 * (attempt as u64 + 1))
}

pub fn is_cloudkit_retryable(err: &Error) -> bool {
    let s = err.to_string().to_lowercase();
    s.contains("changetag")
        || s.contains("conflict")
        || s.contains("server record changed")
        || s.contains("stale")
        || s.contains("record changed")
        || s.contains("oplock")
        || s.contains("op-lock")
        || s.contains("op lock")
        || s.contains("zone_busy")
        || s.contains("cas ")
        || s.contains("retry request")
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
                tokio::time::sleep(retry_delay(attempt)).await;
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
                tokio::time::sleep(retry_delay(attempt)).await;
                engine.sync(false).await?;
            }
            Err(e) => return Err(e),
        }
    }
    unreachable!()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cloudkit_zone_busy_op_lock_is_retryable() {
        let err = Error::Api {
            status: 409,
            body: "ZONE_BUSY: Sync zone CAS Op-Lock failed. Retry request...".into(),
        };
        assert!(is_cloudkit_retryable(&err));
    }

    #[test]
    fn unrelated_api_errors_are_not_retryable() {
        let err = Error::Api {
            status: 400,
            body: "VALIDATING_REFERENCE_ERROR".into(),
        };
        assert!(!is_cloudkit_retryable(&err));
    }
}
