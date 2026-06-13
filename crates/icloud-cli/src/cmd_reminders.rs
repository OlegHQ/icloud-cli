use clap::Subcommand;
use icloud_api::session::SecretsBackend;
use icloud_api::{with_reminders_retry, Result as IResult};

use crate::output::{self, hint, print_ok, print_ok_with, OutputMode};
use crate::{resolve_date_filter, resolve_due_date, OpenReminders, RemindersArgs};

#[derive(Subcommand)]
pub(crate) enum RemindersCmd {
    /// Sync reminders from iCloud into the local database.
    Sync {
        #[command(flatten)]
        args: RemindersArgs,
        /// Delete local DB and re-sync from scratch.
        #[arg(long)]
        force: bool,
    },
    /// Show reminders with smart filters (default: today).
    ///
    /// Filters: today, tomorrow, week, overdue, upcoming, completed, all,
    /// or a date (YYYY-MM-DD).
    #[command(alias = "show")]
    List {
        #[command(flatten)]
        args: RemindersArgs,
        /// Filter: today, tomorrow, week, overdue, upcoming, completed, all, or YYYY-MM-DD.
        #[arg(default_value = None)]
        filter: Option<String>,
        /// Filter by list name (case-insensitive).
        #[arg(short = 'l', long)]
        list: Option<String>,
        /// Include completed reminders (shorthand for filter=all).
        #[arg(short = 'a', long)]
        all: bool,
        /// Only show reminders due on or after this date (YYYY-MM-DD).
        #[arg(long)]
        from: Option<String>,
        /// Only show reminders due on or before this date (YYYY-MM-DD).
        #[arg(long)]
        to: Option<String>,
        /// Maximum number of reminders to show.
        #[arg(short = 'n', long, default_value = "50")]
        limit: usize,
    },
    /// Manage reminder lists. Shows all lists when called without flags.
    #[command(alias = "ls")]
    Lists {
        #[command(flatten)]
        args: RemindersArgs,
        /// List name to operate on (show its contents).
        name: Option<String>,
        /// Rename the list to this value.
        #[arg(short = 'r', long)]
        rename: Option<String>,
        /// Delete the list.
        #[arg(short = 'd', long)]
        delete: bool,
        /// Create the list if it doesn't exist.
        #[arg(long)]
        create: bool,
        /// Skip confirmation prompts.
        #[arg(short = 'f', long)]
        force: bool,
    },
    /// Create a new reminder.
    Add {
        #[command(flatten)]
        args: RemindersArgs,
        /// Target list name (case-insensitive).
        #[arg(short = 'l', long)]
        list: String,
        /// Reminder title (positional or --title).
        title: Option<String>,
        /// Reminder title (alternative to positional).
        #[arg(short = 't', long = "title", conflicts_with = "title")]
        title_flag: Option<String>,
        /// Due date: today, tomorrow, yesterday, or YYYY-MM-DD.
        #[arg(short = 'd', long)]
        due: Option<String>,
        /// Priority: high, medium, low, none.
        #[arg(short = 'p', long)]
        priority: Option<String>,
        /// Notes text.
        #[arg(short = 'n', long)]
        notes: Option<String>,
        /// Parent reminder ID or prefix (for subtasks).
        #[arg(long)]
        parent: Option<String>,
    },
    /// Create multiple reminders at once.
    AddBatch {
        #[command(flatten)]
        args: RemindersArgs,
        /// Target list name.
        #[arg(short = 'l', long)]
        list: String,
        /// Parent reminder ID or prefix.
        #[arg(long)]
        parent: Option<String>,
        /// Reminder titles.
        titles: Vec<String>,
    },
    /// Mark reminders as completed.
    #[command(alias = "done")]
    Complete {
        #[command(flatten)]
        args: RemindersArgs,
        /// Reminder IDs or UUID prefixes.
        ids: Vec<String>,
        /// Preview which reminders would be completed without making changes.
        #[arg(short = 'n', long)]
        dry_run: bool,
    },
    /// Delete reminders.
    #[command(alias = "rm")]
    Delete {
        #[command(flatten)]
        args: RemindersArgs,
        /// Reminder IDs or UUID prefixes.
        ids: Vec<String>,
        /// Preview which reminders would be deleted without making changes.
        #[arg(short = 'n', long)]
        dry_run: bool,
        /// Skip confirmation prompt.
        #[arg(short = 'f', long)]
        force: bool,
    },
    /// Edit a reminder's title, due date, notes, or priority.
    Edit {
        #[command(flatten)]
        args: RemindersArgs,
        /// Reminder ID or UUID prefix.
        id: String,
        /// New title.
        #[arg(short = 't', long)]
        title: Option<String>,
        /// New due date: today, tomorrow, yesterday, or YYYY-MM-DD.
        #[arg(short = 'd', long)]
        due: Option<String>,
        /// Clear the due date.
        #[arg(long, conflicts_with = "due")]
        clear_due: bool,
        /// New notes text.
        #[arg(short = 'n', long)]
        notes: Option<String>,
        /// New priority: high, medium, low, none.
        #[arg(short = 'p', long)]
        priority: Option<String>,
        /// Mark as completed.
        #[arg(long, conflicts_with = "incomplete")]
        complete: bool,
        /// Mark as incomplete.
        #[arg(long, conflicts_with = "complete")]
        incomplete: bool,
    },
}

pub(crate) async fn handle_reminders(
    out: OutputMode,
    secrets: SecretsBackend,
    max_age: u64,
    sub: RemindersCmd,
) -> IResult<()> {
    let json = out.json;
    match sub {
        RemindersCmd::Sync { args, force } => {
            let sp = args.session_path();
            let dp = args.db_path();
            let mut r = OpenReminders::open(&sp, &dp, secrets, force, 0, !out.is_human()).await?;
            let count = r.engine.cache.reminders.len();
            r.save()?;
            print_ok_with(
                json,
                &format!("Synced {count} reminders to {}", dp.display()),
                &serde_json::json!({"db": dp.display().to_string(), "count": count}),
            );
            if out.is_human() {
                hint(&[
                    "icloud reminders list      — list incomplete reminders",
                    "icloud reminders lists     — list all reminder lists",
                ]);
            }
        }

        RemindersCmd::List {
            args,
            filter,
            list,
            all,
            from,
            to,
            limit,
        } => {
            let mut r = OpenReminders::open(
                &args.session_path(),
                &args.db_path(),
                secrets,
                false,
                max_age,
                !out.is_human(),
            )
            .await?;

            // Resolve smart filter into (include_completed, from_date, to_date)
            let (inc_completed, filter_from, filter_to) =
                resolve_date_filter(filter.as_deref(), all);

            // For "completed" filter, only show completed ones
            let show_completed_only = matches!(filter.as_deref(), Some("completed" | "done" | "c"));

            let reminders = r.engine.get_reminders(inc_completed || show_completed_only);
            r.save()?;

            // Merge explicit --from/--to with filter-derived bounds
            let eff_from = from.or(filter_from);
            let eff_to = to.or(filter_to);

            // For "overdue" filter, only include reminders that have a due date in the past
            let is_overdue = matches!(filter.as_deref(), Some("overdue" | "o"));
            // For "upcoming", only include reminders with a due date
            let is_upcoming = matches!(filter.as_deref(), Some("upcoming" | "u"));

            let mut filtered: Vec<_> = reminders
                .into_iter()
                .filter(|r| {
                    list.as_ref()
                        .is_none_or(|lf| r.list_name.eq_ignore_ascii_case(lf))
                })
                .filter(|r| {
                    if show_completed_only {
                        return r.completed;
                    }
                    true
                })
                .filter(|r| {
                    if is_overdue {
                        return r.due.is_some() && !r.completed;
                    }
                    if is_upcoming {
                        return r.due.is_some();
                    }
                    true
                })
                .filter(|r| {
                    if let Some(ref f) = eff_from {
                        r.due.as_ref().is_some_and(|d| d.as_str() >= f.as_str())
                    } else {
                        true
                    }
                })
                .filter(|r| {
                    if let Some(ref t) = eff_to {
                        r.due
                            .as_ref()
                            .is_some_and(|d| d.len() >= 10 && &d[..10] <= t.as_str())
                    } else {
                        true
                    }
                })
                .collect();
            filtered.sort_by(|a, b| a.due.cmp(&b.due));
            let total = filtered.len();
            filtered.truncate(limit);
            output::print_reminders_mode(out, &filtered);
            if out.is_human() {
                if total > limit {
                    eprintln!("Showing {limit} of {total} (use -n to change)");
                }
                if filtered.is_empty() {
                    if list.is_some() {
                        hint(&["icloud reminders lists     — see available lists"]);
                    } else {
                        hint(&["icloud reminders list -a   — include completed"]);
                    }
                }
            }
        }

        RemindersCmd::Lists {
            args,
            name,
            rename,
            delete,
            create,
            force,
        } => {
            let mut r = OpenReminders::open(
                &args.session_path(),
                &args.db_path(),
                secrets,
                false,
                max_age,
                !out.is_human(),
            )
            .await?;

            if let Some(ref list_name) = name {
                if let Some(ref new_name) = rename {
                    let ln = list_name.clone();
                    let nn = new_name.clone();
                    with_reminders_retry(&mut r.engine, |e| {
                        let ln = ln.clone();
                        let nn = nn.clone();
                        Box::pin(async move { e.rename_list(&ln, &nn).await })
                    })
                    .await?;
                    r.save()?;
                    print_ok(json, &format!("Renamed '{list_name}' to '{new_name}'"));
                } else if delete {
                    if !force && !out.no_input && !json {
                        eprint!("Delete list '{list_name}' and all its reminders? [y/N] ");
                        let mut input = String::new();
                        std::io::stdin()
                            .read_line(&mut input)
                            .map_err(|e| icloud_api::Error::Reminders(format!("stdin: {e}")))?;
                        if !input.trim().eq_ignore_ascii_case("y") {
                            eprintln!("Cancelled");
                            return Ok(());
                        }
                    }
                    let ln = list_name.clone();
                    with_reminders_retry(&mut r.engine, |e| {
                        let ln = ln.clone();
                        Box::pin(async move { e.delete_list(&ln).await })
                    })
                    .await?;
                    r.save()?;
                    print_ok(json, &format!("Deleted list '{list_name}'"));
                } else if create {
                    let ln = list_name.clone();
                    with_reminders_retry(&mut r.engine, |e| {
                        let ln = ln.clone();
                        Box::pin(async move { e.create_list(&ln).await })
                    })
                    .await?;
                    r.save()?;
                    print_ok(json, &format!("Created list '{list_name}'"));
                } else {
                    // Show reminders in this list
                    let reminders = r.engine.get_reminders(false);
                    r.save()?;
                    let filtered: Vec<_> = reminders
                        .into_iter()
                        .filter(|r| r.list_name.eq_ignore_ascii_case(list_name))
                        .collect();
                    output::print_reminders_mode(out, &filtered);
                }
            } else {
                // No name — show all lists
                let lists = r.engine.get_lists();
                r.save()?;
                output::print_reminder_lists_mode(out, &lists);
            }
        }

        RemindersCmd::Add {
            args,
            list,
            title,
            title_flag,
            due,
            priority,
            notes,
            parent,
        } => {
            let resolved_title = title
                .or(title_flag)
                .ok_or_else(|| icloud_api::Error::Reminders("missing reminder title".into()))?;
            // Resolve natural-language due dates
            let resolved_due = due.map(|d| resolve_due_date(&d));
            let mut r = OpenReminders::open(
                &args.session_path(),
                &args.db_path(),
                secrets,
                false,
                max_age,
                !out.is_human(),
            )
            .await?;
            let t = resolved_title.clone();
            let list_for_op = list.clone();
            let due_arg = resolved_due.clone();
            let pri_arg = priority.clone();
            let notes_arg = notes.clone();
            let parent_arg = parent.clone();
            let id = with_reminders_retry(&mut r.engine, |e| {
                let title = resolved_title.clone();
                let list = list_for_op.clone();
                let due = due_arg.clone();
                let pri = pri_arg.clone();
                let notes = notes_arg.clone();
                let parent = parent_arg.clone();
                Box::pin(async move {
                    e.add_reminder(
                        &title,
                        &list,
                        due.as_deref(),
                        pri.as_deref(),
                        notes.as_deref(),
                        parent.as_deref(),
                    )
                    .await
                })
            })
            .await?;
            r.save()?;
            print_ok_with(
                json,
                &format!("Added: {t}"),
                &serde_json::json!({"id": id, "title": t, "list": list}),
            );
            if out.is_human() {
                hint(&["icloud reminders list      — see your reminders"]);
            }
        }

        RemindersCmd::AddBatch {
            args,
            list,
            parent,
            titles,
        } => {
            let mut r = OpenReminders::open(
                &args.session_path(),
                &args.db_path(),
                secrets,
                false,
                max_age,
                !out.is_human(),
            )
            .await?;
            let count = titles.len();
            let titles_arg = titles.clone();
            let list_arg = list.clone();
            let parent_arg = parent.clone();
            let ids = with_reminders_retry(&mut r.engine, |e| {
                let titles = titles_arg.clone();
                let list = list_arg.clone();
                let parent = parent_arg.clone();
                Box::pin(async move {
                    e.add_reminders_batch(&titles, &list, parent.as_deref())
                        .await
                })
            })
            .await?;
            r.save()?;
            print_ok_with(
                json,
                &format!("Added {count} reminders"),
                &serde_json::json!({"count": count, "ids": ids}),
            );
        }

        RemindersCmd::Complete { args, ids, dry_run } => {
            if ids.is_empty() {
                return Err(icloud_api::Error::Reminders(
                    "no reminder IDs specified".into(),
                ));
            }
            let mut r = OpenReminders::open(
                &args.session_path(),
                &args.db_path(),
                secrets,
                false,
                max_age,
                !out.is_human(),
            )
            .await?;
            if dry_run {
                for id in &ids {
                    let found = r.engine.cache.find_reminder(id);
                    match found {
                        Some(full) => {
                            let title = r
                                .engine
                                .cache
                                .reminders
                                .get(&full)
                                .map(|d| d.title.as_str())
                                .unwrap_or("?");
                            eprintln!("Would complete: {title} ({full})");
                        }
                        None => eprintln!("Not found: {id}"),
                    }
                }
                r.save()?;
            } else {
                let count = ids.len();
                for id in &ids {
                    let target = id.clone();
                    with_reminders_retry(&mut r.engine, |e| {
                        let id = target.clone();
                        Box::pin(async move { e.complete_reminder(&id).await })
                    })
                    .await?;
                }
                r.save()?;
                print_ok_with(
                    json,
                    &format!("Completed {count} reminder(s)"),
                    &serde_json::json!({"count": count}),
                );
            }
        }

        RemindersCmd::Delete {
            args,
            ids,
            dry_run,
            force,
        } => {
            if ids.is_empty() {
                return Err(icloud_api::Error::Reminders(
                    "no reminder IDs specified".into(),
                ));
            }
            let mut r = OpenReminders::open(
                &args.session_path(),
                &args.db_path(),
                secrets,
                false,
                max_age,
                !out.is_human(),
            )
            .await?;
            if dry_run {
                for id in &ids {
                    let found = r.engine.cache.find_reminder(id);
                    match found {
                        Some(full) => {
                            let title = r
                                .engine
                                .cache
                                .reminders
                                .get(&full)
                                .map(|d| d.title.as_str())
                                .unwrap_or("?");
                            eprintln!("Would delete: {title} ({full})");
                        }
                        None => eprintln!("Not found: {id}"),
                    }
                }
                r.save()?;
            } else {
                if !force && !out.no_input && !json && ids.len() > 1 {
                    eprint!("Delete {} reminders? [y/N] ", ids.len());
                    let mut input = String::new();
                    std::io::stdin()
                        .read_line(&mut input)
                        .map_err(|e| icloud_api::Error::Reminders(format!("stdin: {e}")))?;
                    if !input.trim().eq_ignore_ascii_case("y") {
                        eprintln!("Cancelled");
                        return Ok(());
                    }
                }
                let count = ids.len();
                for id in &ids {
                    let target = id.clone();
                    with_reminders_retry(&mut r.engine, |e| {
                        let id = target.clone();
                        Box::pin(async move { e.delete_reminder(&id).await })
                    })
                    .await?;
                }
                r.save()?;
                print_ok_with(
                    json,
                    &format!("Deleted {count} reminder(s)"),
                    &serde_json::json!({"count": count}),
                );
            }
        }

        RemindersCmd::Edit {
            args,
            id,
            title,
            due,
            clear_due,
            notes,
            priority,
            complete,
            incomplete,
        } => {
            let mut r = OpenReminders::open(
                &args.session_path(),
                &args.db_path(),
                secrets,
                false,
                max_age,
                !out.is_human(),
            )
            .await?;
            if complete {
                let target = id.clone();
                with_reminders_retry(&mut r.engine, |e| {
                    let id = target.clone();
                    Box::pin(async move { e.complete_reminder(&id).await })
                })
                .await?;
                r.save()?;
                print_ok(json, "Completed");
                return Ok(());
            }
            if incomplete {
                let target = id.clone();
                with_reminders_retry(&mut r.engine, |e| {
                    let id = target.clone();
                    Box::pin(async move { e.uncomplete_reminder(&id).await })
                })
                .await?;
                r.save()?;
                print_ok(json, "Marked incomplete");
                return Ok(());
            }
            let resolved_due = due.map(|d| resolve_due_date(&d));
            let target = id.clone();
            let title_arg = title.clone();
            let due_arg = resolved_due.clone();
            let notes_arg = notes.clone();
            let pri_arg = priority.clone();
            with_reminders_retry(&mut r.engine, |e| {
                let id = target.clone();
                let title = title_arg.clone();
                let due = due_arg.clone();
                let notes = notes_arg.clone();
                let pri = pri_arg.clone();
                Box::pin(async move {
                    e.edit_reminder(
                        &id,
                        title.as_deref(),
                        due.as_deref(),
                        clear_due,
                        notes.as_deref(),
                        pri.as_deref(),
                    )
                    .await
                })
            })
            .await?;
            r.save()?;
            print_ok(json, "Updated");
        }
    }
    Ok(())
}
