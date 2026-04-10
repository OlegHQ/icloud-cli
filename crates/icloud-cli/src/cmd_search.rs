use std::path::PathBuf;

use clap::ValueEnum;
use icloud_api::hme::HideMyEmailClient;
use icloud_api::search::{SearchHit, SearchIndex, SearchOptions, SearchService};
use icloud_api::session::{default_search_index_path, load_session, SecretsBackend};
use icloud_bash::pathmap::{classify, normalize_vpath, VfsTarget};

use crate::output::{self, hint, OutputMode};
use crate::{IcloudFsArgs, OpenNotes, OpenReminders};

#[derive(Clone, Copy, Debug, ValueEnum)]
pub(crate) enum SearchServiceArg {
    Notes,
    Reminders,
    #[value(name = "hme", alias = "hide-my-email")]
    HideMyEmail,
}

impl From<SearchServiceArg> for SearchService {
    fn from(value: SearchServiceArg) -> Self {
        match value {
            SearchServiceArg::Notes => SearchService::Notes,
            SearchServiceArg::Reminders => SearchService::Reminders,
            SearchServiceArg::HideMyEmail => SearchService::HideMyEmail,
        }
    }
}

pub(crate) async fn run_search(
    out: OutputMode,
    secrets: SecretsBackend,
    max_age: u64,
    args: IcloudFsArgs,
    query: String,
    service: Option<SearchServiceArg>,
    paths: Vec<String>,
    limit: usize,
    rebuild: bool,
    index: Option<PathBuf>,
) -> icloud_api::Result<()> {
    let session_path = args.session_path();
    let notes_db = args.notes_db_path();
    let reminders_db = args.reminders_db_path();
    let scopes: Vec<String> = paths
        .iter()
        .map(|path| normalize_scope(path))
        .collect::<icloud_api::Result<_>>()?;
    let target_services = requested_services(service, &scopes)?;
    let open_notes = wants_service(&target_services, SearchService::Notes);
    let open_reminders = wants_service(&target_services, SearchService::Reminders);
    let open_hme = wants_service(&target_services, SearchService::HideMyEmail);

    let (mut notes, mut reminders) = match (open_notes, open_reminders) {
        (true, true) => {
            let (notes_result, reminders_result) = tokio::join!(
                OpenNotes::open(
                    &session_path,
                    &notes_db,
                    secrets,
                    false,
                    max_age,
                    !out.is_human()
                ),
                OpenReminders::open(
                    &session_path,
                    &reminders_db,
                    secrets,
                    false,
                    max_age,
                    !out.is_human(),
                ),
            );
            (Some(notes_result?), Some(reminders_result?))
        }
        (true, false) => (
            Some(
                OpenNotes::open(
                    &session_path,
                    &notes_db,
                    secrets,
                    false,
                    max_age,
                    !out.is_human(),
                )
                .await?,
            ),
            None,
        ),
        (false, true) => (
            None,
            Some(
                OpenReminders::open(
                    &session_path,
                    &reminders_db,
                    secrets,
                    false,
                    max_age,
                    !out.is_human(),
                )
                .await?,
            ),
        ),
        (false, false) => (None, None),
    };

    let index = SearchIndex::new(index.unwrap_or_else(default_search_index_path));
    index
        .refresh(
            notes.as_mut().map(|open| &mut open.engine),
            reminders.as_mut().map(|open| &mut open.engine),
            rebuild,
        )
        .await?;

    if open_hme {
        // HideMyEmail has no local cache to reuse, so we always re-fetch the
        // alias list; `refresh_hme` still skips the rebuild when the
        // fingerprint of the returned aliases hasn't changed.
        let session = load_session(&session_path, secrets)?;
        let client = HideMyEmailClient::new(session)?;
        index.refresh_hme(&client, rebuild).await?;
    }

    let hits: Vec<SearchHit> = index.search(
        &target_services,
        &SearchOptions {
            query,
            limit,
            scopes,
            service: service.map(Into::into),
        },
    )?;

    if let Some(notes) = notes.as_mut() {
        notes.save()?;
    }
    if let Some(reminders) = reminders.as_mut() {
        reminders.save()?;
    }
    output::print_search_hits_mode(out, &hits);

    if out.is_human() && hits.is_empty() {
        hint(&[
            "Try a broader query or drop --path to search both services.",
            "Use `icloud search --rebuild ...` if you expect recently changed content.",
        ]);
    }

    Ok(())
}

fn normalize_scope(raw: &str) -> icloud_api::Result<String> {
    let trimmed = raw.trim();
    let without_scheme = trimmed.strip_prefix("icloud:").unwrap_or(trimmed).trim();
    let normalized = normalize_vpath(without_scheme);
    classify_search_scope(&normalized)?;
    Ok(normalized)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ScopeService {
    All,
    Notes,
    Reminders,
    HideMyEmail,
}

fn classify_search_scope(path: &str) -> icloud_api::Result<ScopeService> {
    match classify(path) {
        Ok(VfsTarget::Root) => Ok(ScopeService::All),
        Ok(VfsTarget::NotesRoot | VfsTarget::NotesFolder { .. } | VfsTarget::NotesFile { .. }) => {
            Ok(ScopeService::Notes)
        }
        Ok(
            VfsTarget::RemindersRoot
            | VfsTarget::RemindersList { .. }
            | VfsTarget::RemindersFile { .. },
        ) => Ok(ScopeService::Reminders),
        Ok(
            VfsTarget::HideMyEmailRoot
            | VfsTarget::HideMyEmailAliases
            | VfsTarget::HideMyEmailFile { .. },
        ) => Ok(ScopeService::HideMyEmail),
        Ok(_) | Err(_) => Err(icloud_api::Error::Usage(format!(
            "invalid iCloud search scope '{path}'"
        ))),
    }
}

fn requested_services(
    explicit: Option<SearchServiceArg>,
    scopes: &[String],
) -> icloud_api::Result<Vec<SearchService>> {
    let scope_services: Vec<ScopeService> = scopes
        .iter()
        .map(|scope| classify_search_scope(scope))
        .collect::<icloud_api::Result<_>>()?;

    if let Some(explicit) = explicit {
        let service: SearchService = explicit.into();
        let conflicts = scope_services.iter().any(|scope| match (service, scope) {
            (SearchService::Notes, ScopeService::Reminders | ScopeService::HideMyEmail) => true,
            (SearchService::Reminders, ScopeService::Notes | ScopeService::HideMyEmail) => true,
            (SearchService::HideMyEmail, ScopeService::Notes | ScopeService::Reminders) => true,
            _ => false,
        });
        if conflicts {
            return Err(icloud_api::Error::Usage(format!(
                "--service {} conflicts with the selected search scopes",
                service.as_str()
            )));
        }
        return Ok(vec![service]);
    }

    let wants_notes = scopes.is_empty()
        || scope_services
            .iter()
            .any(|scope| matches!(scope, ScopeService::All | ScopeService::Notes));
    let wants_reminders = scopes.is_empty()
        || scope_services
            .iter()
            .any(|scope| matches!(scope, ScopeService::All | ScopeService::Reminders));
    let wants_hme = scopes.is_empty()
        || scope_services
            .iter()
            .any(|scope| matches!(scope, ScopeService::All | ScopeService::HideMyEmail));

    let mut services = Vec::with_capacity(3);
    if wants_notes {
        services.push(SearchService::Notes);
    }
    if wants_reminders {
        services.push(SearchService::Reminders);
    }
    if wants_hme {
        services.push(SearchService::HideMyEmail);
    }
    Ok(services)
}

fn wants_service(services: &[SearchService], needle: SearchService) -> bool {
    services.contains(&needle)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_scheme_prefixed_scope() {
        assert_eq!(
            normalize_scope("icloud:/Notes/Work/").unwrap(),
            "/Notes/Work"
        );
    }

    #[test]
    fn rejects_host_scope() {
        assert!(normalize_scope("./tmp").is_err());
    }

    #[test]
    fn rejects_non_searchable_scope() {
        // `/Attachments` remains non-searchable (no text content to index).
        assert!(normalize_scope("/Attachments").is_err());
    }

    #[test]
    fn accepts_hide_my_email_scope() {
        assert_eq!(
            normalize_scope("/HideMyEmail").unwrap(),
            "/HideMyEmail"
        );
        assert_eq!(
            requested_services(None, &[String::from("/HideMyEmail")]).unwrap(),
            vec![SearchService::HideMyEmail]
        );
    }

    #[test]
    fn infers_reminders_from_scope() {
        assert_eq!(
            requested_services(None, &[String::from("/Reminders/Home")]).unwrap(),
            vec![SearchService::Reminders]
        );
    }

    #[test]
    fn rejects_conflicting_service_scope() {
        let err = requested_services(
            Some(SearchServiceArg::Notes),
            &[String::from("/Reminders/Home")],
        )
        .unwrap_err();
        assert!(err.to_string().contains("conflicts"));
    }
}
