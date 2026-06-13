//! Centralized output formatting for human, JSON, plain (tab-separated), and quiet modes.
//!
//! Stream contract:
//!   stdout — command results, JSON payloads, success confirmations.
//!   stderr — diagnostics, progress, prompts, hints, dry-run previews.

use icloud_api::notes::models::{Note as NoteModel, NoteFolder};
use icloud_api::reminders::{models::priority_label, Reminder, ReminderList};
use icloud_api::search::SearchHit;
use icloud_api::session::SessionData;

// ── Output mode ──────────────────────────────────────────

/// Controls how output is rendered across all commands.
#[derive(Clone, Copy)]
#[allow(dead_code)]
pub struct OutputMode {
    pub json: bool,
    pub plain: bool,
    pub quiet: bool,
    pub no_color: bool,
    pub no_input: bool,
}

impl OutputMode {
    pub fn is_human(&self) -> bool {
        !self.json && !self.plain && !self.quiet
    }
}

// ── Table printing ───────────────────────────────────────

pub fn print_table_mode(headers: &[&str], rows: &[Vec<String>], mode: OutputMode) {
    if mode.plain {
        // Tab-separated, no headers
        for row in rows {
            println!("{}", row.join("\t"));
        }
        return;
    }
    if rows.is_empty() {
        println!("(none)");
        return;
    }
    let n = headers.len();
    let mut widths: Vec<usize> = headers.iter().map(|h| h.len()).collect();
    for row in rows {
        for (i, cell) in row.iter().enumerate().take(n) {
            // Use unicode width for proper emoji/CJK alignment
            widths[i] = widths[i].max(cell.chars().count());
        }
    }
    let fmt_row = |cells: &[&str]| -> String {
        cells
            .iter()
            .enumerate()
            .map(|(i, c)| {
                let padding = widths[i].saturating_sub(c.chars().count());
                format!("{c}{}", " ".repeat(padding))
            })
            .collect::<Vec<_>>()
            .join("  ")
    };
    println!("{}", fmt_row(headers));
    for row in rows {
        let refs: Vec<&str> = row.iter().map(|s| s.as_str()).collect();
        println!("{}", fmt_row(&refs));
    }
}

pub fn print_kv(pairs: &[(&str, &str)]) {
    let max_key = pairs.iter().map(|(k, _)| k.len()).max().unwrap_or(0);
    for (k, v) in pairs {
        println!("{:>w$}:  {v}", k, w = max_key);
    }
}

pub fn print_json<T: serde::Serialize + ?Sized>(value: &T) {
    println!(
        "{}",
        serde_json::to_string_pretty(value).unwrap_or_else(|_| "null".into())
    );
}

pub fn print_json_compact<T: serde::Serialize + ?Sized>(value: &T) {
    println!(
        "{}",
        serde_json::to_string(value).unwrap_or_else(|_| "null".into())
    );
}

pub fn hint(items: &[&str]) {
    if items.is_empty() {
        return;
    }
    eprintln!();
    for (i, item) in items.iter().enumerate() {
        if i == 0 {
            eprintln!("hint: {item}");
        } else {
            eprintln!("      {item}");
        }
    }
}

// ── Reminders ──────────────────────────────────────────────

pub fn print_reminders_mode(mode: OutputMode, reminders: &[Reminder]) {
    if mode.json {
        print_json(&reminders);
        return;
    }
    if mode.quiet {
        println!("{}", reminders.len());
        return;
    }
    let rows: Vec<Vec<String>> = reminders
        .iter()
        .map(|r| {
            vec![
                r.list_name.clone(),
                r.title.clone(),
                r.due.clone().unwrap_or_default(),
                priority_label(r.priority).to_string(),
                r.id.clone(),
            ]
        })
        .collect();
    print_table_mode(&["LIST", "TITLE", "DUE", "PRI", "ID"], &rows, mode);
}

pub fn print_reminder_lists_mode(mode: OutputMode, lists: &[ReminderList]) {
    if mode.json {
        print_json(&lists);
        return;
    }
    if mode.quiet {
        println!("{}", lists.len());
        return;
    }
    let rows: Vec<Vec<String>> = lists
        .iter()
        .map(|l| vec![l.name.clone(), l.id.clone()])
        .collect();
    print_table_mode(&["NAME", "ID"], &rows, mode);
}

// ── Notes ─────────────────────────────────────────────────

pub fn print_notes_mode(mode: OutputMode, notes: &[NoteModel]) {
    if mode.json {
        print_json(&notes);
        return;
    }
    if mode.quiet {
        println!("{}", notes.len());
        return;
    }
    let rows: Vec<Vec<String>> = notes
        .iter()
        .map(|n| {
            vec![
                n.folder_name.clone(),
                n.title.clone(),
                n.modified.clone().unwrap_or_default(),
                n.id.clone(),
            ]
        })
        .collect();
    print_table_mode(&["FOLDER", "TITLE", "MODIFIED", "ID"], &rows, mode);
}

pub fn print_note_folders_mode(mode: OutputMode, folders: &[NoteFolder]) {
    if mode.json {
        print_json(&folders);
        return;
    }
    if mode.quiet {
        println!("{}", folders.len());
        return;
    }
    if folders.is_empty() {
        println!("(none)");
        return;
    }
    if mode.plain {
        for f in folders {
            let parent = f.parent_id.as_deref().unwrap_or("");
            println!("{}\t{}\t{}", f.name, f.id, parent);
        }
        return;
    }
    // Build tree: collect children per parent, then DFS print.
    use std::collections::HashMap;
    let by_id: HashMap<&str, &NoteFolder> = folders.iter().map(|f| (f.id.as_str(), f)).collect();
    let mut children: HashMap<&str, Vec<&NoteFolder>> = HashMap::new();
    let mut roots: Vec<&NoteFolder> = Vec::new();
    for f in folders {
        match &f.parent_id {
            Some(pid) if by_id.contains_key(pid.as_str()) => {
                children.entry(pid.as_str()).or_default().push(f);
            }
            _ => roots.push(f),
        }
    }
    roots.sort_by(|a, b| a.name.cmp(&b.name));
    for c in children.values_mut() {
        c.sort_by(|a, b| a.name.cmp(&b.name));
    }
    fn print_tree(
        out: &mut Vec<Vec<String>>,
        folder: &NoteFolder,
        children: &HashMap<&str, Vec<&NoteFolder>>,
        depth: usize,
    ) {
        let prefix = if depth == 0 {
            String::new()
        } else {
            format!("{}└ ", "  ".repeat(depth - 1))
        };
        out.push(vec![
            format!("{}{}", prefix, folder.name),
            folder.id.clone(),
        ]);
        if let Some(kids) = children.get(folder.id.as_str()) {
            for kid in kids {
                print_tree(out, kid, children, depth + 1);
            }
        }
    }
    let mut rows = Vec::new();
    for root in &roots {
        print_tree(&mut rows, root, &children, 0);
    }
    print_table_mode(&["NAME", "ID"], &rows, mode);
}

// ── Search ───────────────────────────────────────────────

pub fn print_search_hits_mode(mode: OutputMode, hits: &[SearchHit]) {
    if mode.json {
        print_json(hits);
        return;
    }
    if mode.quiet {
        println!("{}", hits.len());
        return;
    }
    for (idx, hit) in hits.iter().enumerate() {
        if idx > 0 {
            println!();
        }

        let kind = match hit.kind {
            icloud_api::search::SearchResultKind::NoteFolder => "note_folder",
            icloud_api::search::SearchResultKind::NoteFile => "note_file",
            icloud_api::search::SearchResultKind::ReminderList => "reminder_list",
            icloud_api::search::SearchResultKind::ReminderFile => "reminder_file",
            icloud_api::search::SearchResultKind::HideMyEmailAlias => "hide_my_email_alias",
        };
        println!("{}", hit.name);
        println!("  type: {kind}");
        println!("  path: {}", hit.path);
        if let Some(container) = hit.container.as_deref().filter(|value| !value.is_empty()) {
            println!("  list: {container}");
        }
        if matches!(hit.kind, icloud_api::search::SearchResultKind::ReminderFile) {
            if let Some(due) = hit.due.as_deref().filter(|value| !value.is_empty()) {
                println!("  due: {due}");
            }
        }

        if let Some(snippet) = hit.snippet.as_deref() {
            if !snippet.is_empty() {
                println!("  snippet: {snippet}");
            }
        }
    }
}

// ── Session / Whoami ───────────────────────────────────────

pub fn print_whoami(
    mode: OutputMode,
    session_path: &std::path::Path,
    session: &SessionData,
    ok: bool,
) {
    if mode.json {
        print_json(&serde_json::json!({
            "session": session_path,
            "validates": ok,
            "dsid": session.dsid,
            "apple_id": session.apple_id,
            "ck_base_url": session.ck_base_url,
        }));
        return;
    }
    let status = if ok { "ok" } else { "failed" };
    let dsid = session.dsid.as_deref().unwrap_or("-");
    let apple_id = session.apple_id.as_deref().unwrap_or("-");
    let path_str = session_path.display().to_string();
    if mode.quiet {
        println!("{status}");
        return;
    }
    if mode.plain {
        println!("{path_str}\t{apple_id}\t{status}\t{dsid}");
        return;
    }
    print_kv(&[
        ("Session", &path_str),
        ("Apple ID", apple_id),
        ("Status", status),
        ("DSID", dsid),
    ]);
}

// ── HME ────────────────────────────────────────────────────

pub fn print_hme_generate(json: bool, email: Option<&str>) {
    if json {
        print_json(&serde_json::json!({ "email": email }));
        return;
    }
    match email {
        Some(e) => println!("{e}"),
        None => println!("(no email generated)"),
    }
}

pub fn print_hme_action(json: bool, action: &str, ok: bool) {
    if json {
        print_json_compact(&serde_json::json!({ "ok": ok }));
    } else if ok {
        println!("{action}: ok");
    } else {
        eprintln!("{action}: failed");
    }
}

pub fn print_hme_list_mode(mode: OutputMode, raw: &serde_json::Value) {
    if mode.json {
        print_json(raw);
        return;
    }
    let aliases = icloud_api::hme::HmeAlias::parse_list_response(raw);
    if mode.quiet {
        println!("{}", aliases.len());
        return;
    }
    if mode.plain {
        for a in &aliases {
            let active = match a.is_active {
                Some(true) => "active",
                Some(false) => "inactive",
                None => "",
            };
            let forward = a.forward_to_email.as_deref().unwrap_or("");
            let origin = a.origin.as_deref().unwrap_or("");
            let created = a
                .create_timestamp
                .and_then(chrono::DateTime::<chrono::Utc>::from_timestamp_millis)
                .map(|dt| dt.to_rfc3339())
                .unwrap_or_default();
            println!(
                "{}\t{}\t{}\t{}\t{}\t{}\t{}",
                a.anonymous_id, a.hme, a.label, forward, active, origin, created
            );
        }
        return;
    }
    let rows: Vec<Vec<String>> = aliases
        .iter()
        .map(|a| {
            let active = match a.is_active {
                Some(true) => "yes",
                Some(false) => "no",
                None => "?",
            };
            vec![
                a.label.clone(),
                a.hme.clone(),
                a.forward_to_email.clone().unwrap_or_default(),
                active.to_string(),
                a.anonymous_id.clone(),
            ]
        })
        .collect();
    print_table_mode(&["LABEL", "EMAIL", "FORWARD", "ACTIVE", "ID"], &rows, mode);
}

// ── Mutation confirmations ─────────────────────────────────

pub fn print_ok(json: bool, msg: &str) {
    if json {
        print_json_compact(&serde_json::json!({ "ok": true }));
    } else {
        println!("{msg}");
    }
}

pub fn print_ok_with(json: bool, msg: &str, extra: &serde_json::Value) {
    if json {
        let mut obj = serde_json::json!({ "ok": true });
        if let (Some(a), Some(b)) = (obj.as_object_mut(), extra.as_object()) {
            a.extend(b.iter().map(|(k, v)| (k.clone(), v.clone())));
        }
        print_json_compact(&obj);
    } else {
        println!("{msg}");
    }
}
