use std::path::PathBuf;

use clap::Subcommand;
use icloud_api::session::SecretsBackend;
use icloud_api::{with_notes_retry, Result as IResult};

use crate::output::{self, hint, print_json, print_ok, print_ok_with, OutputMode};
use crate::{read_body_or_stdin, sync_spinner, NotesArgs, OpenNotes};

#[derive(Subcommand)]
pub(crate) enum NotesCmd {
    /// Sync notes metadata from iCloud.
    Sync {
        #[command(flatten)]
        args: NotesArgs,
        /// Delete local DB and re-sync from scratch.
        #[arg(long)]
        force: bool,
    },
    /// List notes (title, folder, date).
    List {
        #[command(flatten)]
        args: NotesArgs,
        /// Filter by folder name (case-insensitive).
        #[arg(short = 'f', long)]
        folder: Option<String>,
        /// Maximum number of notes to show.
        #[arg(short = 'n', long, default_value = "50")]
        limit: usize,
    },
    /// Manage note folders. Shows all folders when called without flags.
    Folders {
        #[command(flatten)]
        args: NotesArgs,
        /// Delete the folder (and its notes).
        #[arg(short = 'd', long, conflicts_with = "create")]
        delete: bool,
        /// Create the folder if it doesn't exist.
        #[arg(long)]
        create: bool,
        /// Folder name to operate on.
        name: Option<String>,
        /// Skip confirmation prompts.
        #[arg(short = 'f', long)]
        force: bool,
    },
    /// Show a note's full body as Markdown.
    Get {
        #[command(flatten)]
        args: NotesArgs,
        /// Note ID or title prefix.
        id: String,
        /// Dump raw protobuf attribute runs instead of markdown.
        #[arg(long)]
        debug: bool,
    },
    /// Create a new note from Markdown (pass --body or pipe stdin).
    Create {
        #[command(flatten)]
        args: NotesArgs,
        /// Target folder name.
        #[arg(short = 'f', long, default_value = "Notes")]
        folder: String,
        /// Markdown body, taken verbatim. Shells do not interpret escape sequences in
        /// argument values; for multi-line bodies, pipe stdin instead:
        ///   printf '# Title\n\nBody line 1\nBody line 2\n' | icloud notes create --folder Notes
        #[arg(long, verbatim_doc_comment)]
        body: Option<String>,
    },
    /// Update a note's body from Markdown.
    Update {
        #[command(flatten)]
        args: NotesArgs,
        /// Note ID or title prefix.
        id: String,
        /// New markdown body (reads stdin if omitted).
        #[arg(long)]
        body: Option<String>,
    },
    /// Delete a note (moves to Trash).
    #[command(alias = "rm")]
    Delete {
        #[command(flatten)]
        args: NotesArgs,
        /// Note ID or title prefix.
        id: String,
    },
    /// Move a note to a different folder.
    Move {
        #[command(flatten)]
        args: NotesArgs,
        /// Note ID or title prefix.
        id: String,
        /// Target folder name.
        #[arg(short = 'f', long)]
        folder: String,
    },
    /// Search notes by title or snippet (fuzzy).
    Search {
        #[command(flatten)]
        args: NotesArgs,
        /// Search query.
        query: String,
        /// Filter by folder name.
        #[arg(short = 'f', long)]
        folder: Option<String>,
        /// Maximum results.
        #[arg(short = 'n', long, default_value = "50")]
        limit: usize,
    },
    /// Export notes as Markdown files.
    Export {
        #[command(flatten)]
        args: NotesArgs,
        /// Filter by folder name.
        #[arg(short = 'f', long)]
        folder: Option<String>,
        /// Output directory (default: ~/Desktop/notes/).
        #[arg(short = 'o', long)]
        path: Option<PathBuf>,
    },
}

pub(crate) async fn handle_notes(
    out: OutputMode,
    secrets: SecretsBackend,
    max_age: u64,
    sub: NotesCmd,
) -> IResult<()> {
    let json = out.json;
    match sub {
        NotesCmd::Sync { args, force } => {
            let sp = args.session_path();
            let dp = args.db_path();
            let mut r = OpenNotes::open(&sp, &dp, secrets, force, 0, !out.is_human()).await?;
            let count = r.engine.cache.notes.len();
            r.save()?;
            print_ok_with(
                json,
                &format!("Synced {count} notes to {}", dp.display()),
                &serde_json::json!({"db": dp.display().to_string(), "count": count}),
            );
            if out.is_human() {
                hint(&[
                    "icloud notes list          — list notes",
                    "icloud notes folders       — list folders",
                ]);
            }
        }

        NotesCmd::List {
            args,
            folder,
            limit,
        } => {
            let mut r = OpenNotes::open(
                &args.session_path(),
                &args.db_path(),
                secrets,
                false,
                max_age,
                !out.is_human(),
            )
            .await?;
            let notes = r.engine.get_notes();
            r.save()?;
            let mut filtered: Vec<_> = notes
                .into_iter()
                .filter(|n| {
                    folder
                        .as_ref()
                        .is_none_or(|f| n.folder_name.eq_ignore_ascii_case(f))
                })
                .collect();
            filtered.sort_by(|a, b| b.modified.cmp(&a.modified));
            let total = filtered.len();
            filtered.truncate(limit);
            output::print_notes_mode(out, &filtered);
            if out.is_human() && total > limit {
                eprintln!("Showing {limit} of {total} (use -n to change)");
            }
        }

        NotesCmd::Folders {
            args,
            delete,
            create,
            name,
            force,
        } => {
            let mut r = OpenNotes::open(
                &args.session_path(),
                &args.db_path(),
                secrets,
                false,
                max_age,
                !out.is_human(),
            )
            .await?;
            if let Some(ref folder_name) = name {
                if delete {
                    if !force && !out.no_input && !json {
                        eprint!("Delete folder '{folder_name}' and all its notes? [y/N] ");
                        let mut input = String::new();
                        std::io::stdin()
                            .read_line(&mut input)
                            .map_err(|e| icloud_api::Error::Notes(format!("stdin: {e}")))?;
                        if !input.trim().eq_ignore_ascii_case("y") {
                            eprintln!("Cancelled");
                            return Ok(());
                        }
                    }
                    let notes_in_folder: Vec<String> = r
                        .engine
                        .get_notes()
                        .into_iter()
                        .filter(|n| n.folder_name.eq_ignore_ascii_case(folder_name))
                        .map(|n| n.id)
                        .collect();
                    let count = notes_in_folder.len();
                    for note_id in &notes_in_folder {
                        let target = note_id.clone();
                        with_notes_retry(&mut r.engine, |e| {
                            let id = target.clone();
                            Box::pin(async move { e.delete_note(&id).await })
                        })
                        .await?;
                    }
                    let target_folder = folder_name.clone();
                    with_notes_retry(&mut r.engine, |e| {
                        let folder = target_folder.clone();
                        Box::pin(async move { e.delete_folder(&folder).await })
                    })
                    .await?;
                    r.save()?;
                    print_ok_with(
                        json,
                        &format!("Deleted folder '{folder_name}' ({count} notes)"),
                        &serde_json::json!({"folder": folder_name, "notes_deleted": count}),
                    );
                } else if create {
                    let fn_arg = folder_name.clone();
                    let id = with_notes_retry(&mut r.engine, |e| {
                        let n = fn_arg.clone();
                        Box::pin(async move { e.create_folder(&n).await })
                    })
                    .await?;
                    r.save()?;
                    print_ok_with(
                        json,
                        &format!("Created folder '{folder_name}'"),
                        &serde_json::json!({"folder": folder_name, "id": id}),
                    );
                } else {
                    let notes = r.engine.get_notes();
                    r.save()?;
                    let filtered: Vec<_> = notes
                        .into_iter()
                        .filter(|n| n.folder_name.eq_ignore_ascii_case(folder_name))
                        .collect();
                    output::print_notes_mode(out, &filtered);
                }
            } else {
                let folders = r.engine.get_folders();
                r.save()?;
                output::print_note_folders_mode(out, &folders);
            }
        }

        NotesCmd::Get { args, id, debug } => {
            let mut r = OpenNotes::open(
                &args.session_path(),
                &args.db_path(),
                secrets,
                false,
                max_age,
                !out.is_human(),
            )
            .await?;
            let record_name = r
                .engine
                .cache
                .find_note(&id)
                .ok_or_else(|| icloud_api::Error::Notes(format!("note '{id}' not found")))?;
            if debug {
                let raw = r.engine.fetch_raw(&record_name).await?;
                print!("{}", serde_json::to_string_pretty(&raw).unwrap_or_default());
                r.save()?;
                return Ok(());
            }
            let md = r.engine.fetch_body(&record_name).await?;
            r.save()?;
            if json {
                let mut note_data = r
                    .engine
                    .get_notes()
                    .into_iter()
                    .find(|n| n.id == record_name);
                if let Some(ref mut n) = note_data {
                    n.body = Some(md);
                    print_json(n);
                } else {
                    print_json(&serde_json::json!({"id": record_name, "body": md}));
                }
            } else {
                print!("{md}");
            }
        }

        NotesCmd::Create { args, folder, body } => {
            let md = read_body_or_stdin(body)?;
            let mut r = OpenNotes::open(
                &args.session_path(),
                &args.db_path(),
                secrets,
                false,
                max_age,
                !out.is_human(),
            )
            .await?;
            let folder_name = folder.clone();
            let folder_arg = folder.clone();
            let md_arg = md.clone();
            let id = if md.starts_with("RAW:") {
                with_notes_retry(&mut r.engine, |e| {
                    let body = md_arg.clone();
                    let folder = folder_arg.clone();
                    Box::pin(async move {
                        e.create_note_raw(
                            "RawTest",
                            "raw test",
                            body.trim_start_matches("RAW:").trim(),
                            &folder,
                        )
                        .await
                    })
                })
                .await?
            } else {
                with_notes_retry(&mut r.engine, |e| {
                    let body = md_arg.clone();
                    let folder = folder_arg.clone();
                    Box::pin(async move { e.create_note(&body, &folder).await })
                })
                .await?
            };
            let title = md
                .lines()
                .next()
                .unwrap_or("Untitled")
                .trim_start_matches('#')
                .trim()
                .to_string();
            r.save()?;
            print_ok_with(
                json,
                &format!("Created in folder '{folder_name}'"),
                &serde_json::json!({"id": id, "folder": folder_name, "title": title}),
            );
        }

        NotesCmd::Update { args, id, body } => {
            let md = read_body_or_stdin(body)?;
            let mut r = OpenNotes::open(
                &args.session_path(),
                &args.db_path(),
                secrets,
                false,
                max_age,
                !out.is_human(),
            )
            .await?;
            let target = id.clone();
            with_notes_retry(&mut r.engine, |e| {
                let id = target.clone();
                let md = md.clone();
                Box::pin(async move { e.update_note(&id, &md).await })
            })
            .await?;
            r.save()?;
            print_ok(json, "Updated");
        }

        NotesCmd::Delete { args, id } => {
            let mut r = OpenNotes::open(
                &args.session_path(),
                &args.db_path(),
                secrets,
                false,
                max_age,
                !out.is_human(),
            )
            .await?;
            let target = id.clone();
            with_notes_retry(&mut r.engine, |e| {
                let id = target.clone();
                Box::pin(async move { e.delete_note(&id).await })
            })
            .await?;
            r.save()?;
            print_ok(json, "Deleted");
        }

        NotesCmd::Move { args, id, folder } => {
            let mut r = OpenNotes::open(
                &args.session_path(),
                &args.db_path(),
                secrets,
                false,
                max_age,
                !out.is_human(),
            )
            .await?;
            let folder_name = folder.clone();
            let target = id.clone();
            let folder_arg = folder.clone();
            with_notes_retry(&mut r.engine, |e| {
                let id = target.clone();
                let folder = folder_arg.clone();
                Box::pin(async move { e.move_note(&id, &folder).await })
            })
            .await?;
            r.save()?;
            print_ok_with(
                json,
                &format!("Moved to '{folder_name}'"),
                &serde_json::json!({"folder": folder_name}),
            );
        }

        NotesCmd::Search {
            args,
            query,
            folder,
            limit,
        } => {
            let mut r = OpenNotes::open(
                &args.session_path(),
                &args.db_path(),
                secrets,
                false,
                max_age,
                !out.is_human(),
            )
            .await?;
            let notes = r.engine.get_notes();
            r.save()?;
            let q = query.to_ascii_lowercase();
            let mut matched: Vec<_> = notes
                .into_iter()
                .filter(|n| {
                    folder
                        .as_ref()
                        .is_none_or(|f| n.folder_name.eq_ignore_ascii_case(f))
                })
                .filter(|n| {
                    n.title.to_ascii_lowercase().contains(&q)
                        || n.snippet
                            .as_ref()
                            .is_some_and(|s| s.to_ascii_lowercase().contains(&q))
                })
                .collect();
            matched.sort_by(|a, b| b.modified.cmp(&a.modified));
            let total = matched.len();
            matched.truncate(limit);
            output::print_notes_mode(out, &matched);
            if out.is_human() && total > limit {
                eprintln!("Showing {limit} of {total} (use -n to change)");
            }
        }

        NotesCmd::Export { args, folder, path } => {
            let mut r = OpenNotes::open(
                &args.session_path(),
                &args.db_path(),
                secrets,
                false,
                max_age,
                !out.is_human(),
            )
            .await?;
            let notes = r.engine.get_notes();
            let filtered: Vec<_> = notes
                .into_iter()
                .filter(|n| {
                    folder
                        .as_ref()
                        .is_none_or(|f| n.folder_name.eq_ignore_ascii_case(f))
                })
                .collect();
            let out_dir = path.unwrap_or_else(|| {
                let home = directories::UserDirs::new()
                    .and_then(|u| u.desktop_dir().map(|d| d.to_path_buf()))
                    .unwrap_or_else(|| PathBuf::from("."));
                home.join("notes")
            });
            std::fs::create_dir_all(&out_dir)
                .map_err(|e| icloud_api::Error::Notes(format!("create dir: {e}")))?;
            let count = filtered.len();
            let spinner = sync_spinner("notes export", !out.is_human());
            for note in &filtered {
                // Create subfolder per folder name
                let folder_dir = out_dir.join(sanitize_filename(&note.folder_name));
                std::fs::create_dir_all(&folder_dir)
                    .map_err(|e| icloud_api::Error::Notes(format!("create dir: {e}")))?;
                let filename = format!("{}.md", sanitize_filename(&note.title));
                let filepath = folder_dir.join(&filename);
                // Fetch body for each note
                match r.engine.fetch_body(&note.id).await {
                    Ok(md) => {
                        std::fs::write(&filepath, md).map_err(|e| {
                            icloud_api::Error::Notes(format!("write {}: {e}", filepath.display()))
                        })?;
                    }
                    Err(e) => {
                        eprintln!("warning: skip '{}': {e}", note.title);
                    }
                }
            }
            if let Some(sp) = spinner {
                sp.finish_and_clear();
            }
            r.save()?;
            print_ok_with(
                json,
                &format!("Exported {count} notes to {}", out_dir.display()),
                &serde_json::json!({"path": out_dir.display().to_string(), "count": count}),
            );
        }
    }
    Ok(())
}

/// Sanitize a string for use as a filename (replace path-unsafe characters).
fn sanitize_filename(name: &str) -> String {
    name.chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            _ => c,
        })
        .collect::<String>()
        .trim()
        .to_string()
}
