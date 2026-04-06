pub mod apple_srp;
pub mod auth;
pub mod cloudkit;
pub mod error;
pub mod hme;
pub mod http;
pub mod notes;
pub mod reminders;
pub mod session;
pub mod store;
pub mod title_doc;

pub use auth::{AuthFlow, TrustedPhone, TwoFactorInfo};
pub use cloudkit::CloudKitClient;
pub use error::{Error, JsonErrorReport, Result};
pub use hme::HideMyEmailClient;
pub use notes::{NoteDocument, NotesSyncEngine, NotesStore};
pub use session::{SessionCookie, SessionData};
pub use store::is_cache_fresh;

/// Truncate a string to at most `n` bytes on a char boundary.
pub(crate) fn truncate(s: &str, n: usize) -> &str {
    if s.len() <= n {
        s
    } else {
        let mut end = n;
        while end > 0 && !s.is_char_boundary(end) {
            end -= 1;
        }
        &s[..end]
    }
}
