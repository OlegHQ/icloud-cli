//! Bidirectional Markdown ↔ NoteDocument conversion.

use std::collections::{HashMap, HashSet};

use crate::error::Result;

use super::models::*;
use super::table::TableData;

type LinkResolver<'a> = dyn Fn(&str) -> Option<String> + 'a;

// ── Title → filename stem ───────────────────────────────

const DIV_SLASH: char = '\u{2215}'; // ∕

/// Escape `/` and `\0` in a title for use inside a single path segment (filename stem).
pub fn title_to_filename_stem(title: &str) -> String {
    let mut s: String = title
        .chars()
        .map(|c| match c {
            '/' => DIV_SLASH,
            '\0' => ' ',
            c => c,
        })
        .collect();
    // Collapse runs of whitespace and trim
    let mut t = String::with_capacity(s.len());
    let mut prev_space = true;
    for c in s.chars() {
        let is_space = c.is_whitespace();
        if is_space {
            if !prev_space {
                t.push(' ');
            }
            prev_space = true;
        } else {
            t.push(c);
            prev_space = false;
        }
    }
    if t.ends_with(' ') {
        t.pop();
    }
    s = t;
    if s.is_empty() {
        return "Untitled".to_string();
    }
    // Truncate to 255 bytes
    if s.len() > 255 {
        let mut end = 255;
        while end > 0 && !s.is_char_boundary(end) {
            end -= 1;
        }
        s.truncate(end);
    }
    s
}

/// Build a unique `.md` filename inside a folder, appending ` (2)`, ` (3)`, … on collision.
///
/// `existing` must contain lowercased filenames for O(1) collision checks.
pub fn disambiguate_filename(stem: &str, existing: &HashSet<String>) -> String {
    let base = format!("{stem}.md");
    if !existing.contains(&base.to_ascii_lowercase()) {
        return base;
    }
    let mut n = 2u32;
    loop {
        let candidate = format!("{stem} ({n}).md");
        if !existing.contains(&candidate.to_ascii_lowercase()) {
            return candidate;
        }
        n += 1;
    }
}

/// Map filename (with `.md`) back to a title string (reverse U+2215 → `/`).
pub fn filename_to_title(filename: &str) -> String {
    let stem = filename.strip_suffix(".md").unwrap_or(filename);
    stem.replace(DIV_SLASH, "/")
}

// ── UTI ↔ Extension mapping ─────────────────────────────

fn uti_to_extension(uti: &str) -> &'static str {
    match uti {
        s if s.starts_with("public.png") => ".png",
        s if s.starts_with("public.jpeg") => ".jpg",
        s if s.starts_with("public.tiff") => ".tiff",
        s if s.starts_with("public.heic") => ".heic",
        s if s.starts_with("public.image") => ".png",
        s if s.starts_with("com.adobe.pdf") => ".pdf",
        s if s.starts_with("com.apple.m4a-audio") => ".m4a",
        s if s.starts_with("public.mp3") => ".mp3",
        _ => ".bin",
    }
}

fn extension_to_uti(ext: &str) -> String {
    match ext {
        ".png" => "public.png".into(),
        ".jpg" | ".jpeg" => "public.jpeg".into(),
        ".tiff" => "public.tiff".into(),
        ".heic" => "public.heic".into(),
        ".pdf" => "com.adobe.pdf".into(),
        ".m4a" => "com.apple.m4a-audio".into(),
        ".mp3" => "public.mp3".into(),
        _ => format!("dyn.{}", ext.trim_start_matches('.')),
    }
}

/// Derive the file extension from an original filename, falling back to UTI.
fn attachment_extension<'a>(title: Option<&'a str>, uti: &'a str) -> &'a str {
    title
        .and_then(|t| t.rfind('.').map(|i| &t[i..]))
        .unwrap_or_else(|| uti_to_extension(uti))
}

/// Returns true if a UTI represents an image type.
pub(crate) fn is_image_uti(uti: &str) -> bool {
    uti.starts_with("public.png")
        || uti.starts_with("public.jpeg")
        || uti.starts_with("public.image")
        || uti.starts_with("public.tiff")
        || uti.starts_with("public.heic")
}

// ── NoteDocument → Markdown ───────────────────────────────

/// Returns true if two runs can be merged: same inline formatting AND same paragraph style.
/// We check paragraph style too because merging across style changes (e.g. title→checklist)
/// would lose the style transition when the merged run spans a newline.
fn can_merge_runs(a: &AttributeRun, b: &AttributeRun) -> bool {
    a.font == b.font
        && a.underlined == b.underlined
        && a.strikethrough == b.strikethrough
        && a.link == b.link
        && a.attachment.is_none()
        && b.attachment.is_none()
        && a.style.style_type == b.style.style_type
        && a.style.block_quote == b.style.block_quote
        && a.style.indent == b.style.indent
        && checklist_eq(&a.style.checklist, &b.style.checklist)
}

fn checklist_eq(a: &Option<ChecklistInfo>, b: &Option<ChecklistInfo>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(x), Some(y)) => x.done == y.done,
        _ => false,
    }
}

/// Merge consecutive runs with identical inline formatting so that markdown markers
/// don't land mid-word. Apple Notes often splits runs at arbitrary character boundaries
/// (CRDT artefact) even when the formatting is the same.
fn normalize_runs(doc: &NoteDocument) -> NoteDocument {
    if doc.runs.len() <= 1 {
        return doc.clone();
    }
    let mut merged: Vec<AttributeRun> = Vec::with_capacity(doc.runs.len());
    for run in &doc.runs {
        if let Some(last) = merged.last_mut() {
            if can_merge_runs(last, run) {
                last.length += run.length;
                // Keep the paragraph style of the later run (it's closer to the newline,
                // which is what determines the paragraph style in Apple Notes).
                last.style = run.style.clone();
                continue;
            }
        }
        merged.push(run.clone());
    }
    NoteDocument {
        text: doc.text.clone(),
        runs: merged,
    }
}

/// Resolved attachment content for rendering.
pub enum AttachmentContent {
    /// Table with decoded cell data.
    Table(TableData),
    /// Image attachment: (UTI, optional original filename).
    Image(String, Option<String>),
    /// File/audio attachment: (UTI, optional original filename).
    File(String, Option<String>),
}

/// Convert a NoteDocument to Markdown.
///
/// `attachments` maps attachment identifier → resolved content.
/// Pass an empty map if attachment data isn't available.
pub fn to_markdown(doc: &NoteDocument) -> String {
    to_markdown_with_attachments(doc, &HashMap::new(), None::<fn(&str) -> Option<String>>)
}

/// Convert a NoteDocument to Markdown with resolved attachment content.
pub fn to_markdown_with_attachments(
    doc: &NoteDocument,
    attachments: &HashMap<String, AttachmentContent>,
    link_resolver: Option<impl Fn(&str) -> Option<String>>,
) -> String {
    if doc.text.is_empty() {
        return String::new();
    }
    let lr: Option<&LinkResolver<'_>> = link_resolver.as_ref().map(|f| f as &LinkResolver<'_>);
    // Normalize: merge adjacent runs with same inline formatting to avoid mid-word markers.
    let doc = &normalize_runs(doc);
    let mut out = String::with_capacity(doc.text.len() * 2);

    // Walk runs. Each run covers `run.length` chars. We track:
    //   - `byte_pos`: byte offset into doc.text (for slicing)
    //   - `char_count`: chars consumed in current run
    //   - `para_start`: byte offset of current paragraph's start
    //   - `para_runs`: (byte_offset_in_para, run_ref) spans within this paragraph
    let mut run_idx = 0usize;
    let mut char_count = 0usize; // chars consumed in current run
    let mut para_start = 0usize; // byte offset of paragraph start
    let mut para_runs: Vec<(usize, &AttributeRun)> = Vec::new(); // (byte offset in para, run)
                                                                 // Stack of (indent, counter) for nested numbered lists.
                                                                 // When indent increases, push a new level. When it decreases, pop back.
    let mut list_stack: Vec<(i32, usize)> = Vec::new();

    for (byte_pos, ch) in doc.text.char_indices() {
        let current_run = doc.runs.get(run_idx);

        if ch == '\n' {
            let para = &doc.text[para_start..byte_pos];
            // Push trailing run span if not already recorded at this start
            if let Some(run) = current_run {
                let byte_in_para = byte_pos - para_start;
                if para_runs.last().is_none_or(|&(_, r)| !std::ptr::eq(r, run)) {
                    // Find the last valid char boundary at or before byte_in_para - 1
                    let offset = byte_in_para.saturating_sub(1);
                    let offset = (0..=offset)
                        .rev()
                        .find(|&i| para.is_char_boundary(i))
                        .unwrap_or(0);
                    // Only push if offset is after the last entry (don't shadow earlier runs)
                    let last_offset = para_runs.last().map_or(0, |&(o, _)| o);
                    if offset < para.len() && offset > last_offset {
                        para_runs.push((offset, run));
                    }
                }
            }
            let style = dominant_para_style(para, &para_runs);
            let list_number = advance_list_counter(&mut list_stack, &style);
            emit_paragraph(
                &mut out,
                para,
                &para_runs,
                &style,
                list_number,
                attachments,
                lr,
            );
            out.push('\n');
            para_runs.clear();
            para_start = byte_pos + ch.len_utf8();
        } else {
            // Start a new run span when the run changes
            if let Some(run) = current_run {
                let byte_in_para = byte_pos - para_start;
                if para_runs.last().is_none_or(|&(_, r)| !std::ptr::eq(r, run)) {
                    para_runs.push((byte_in_para, run));
                }
            }
        }

        // Advance run tracking
        if let Some(run) = current_run {
            char_count += 1;
            if char_count >= run.length {
                run_idx += 1;
                char_count = 0;
            }
        }
    }

    // Emit any trailing paragraph (text not ending with '\n')
    if para_start < doc.text.len() {
        let para = &doc.text[para_start..];
        if let Some(run) = doc.runs.get(run_idx).or_else(|| doc.runs.last()) {
            let byte_in_para = doc.text.len() - para_start;
            if para_runs.last().is_none_or(|&(_, r)| !std::ptr::eq(r, run)) {
                para_runs.push((byte_in_para, run));
            }
        }
        let style = dominant_para_style(para, &para_runs);
        let list_number = advance_list_counter(&mut list_stack, &style);
        emit_paragraph(
            &mut out,
            para,
            &para_runs,
            &style,
            list_number,
            attachments,
            lr,
        );
        out.push('\n');
    }

    out
}

/// Advance the numbered-list counter stack and return the current item number.
/// Returns 0 for non-numbered-list paragraphs.
fn advance_list_counter(stack: &mut Vec<(i32, usize)>, style: &ParagraphStyle) -> usize {
    if style.style_type != StyleType::NumberedList {
        stack.clear();
        return 0;
    }
    let indent = style.indent;
    // Pop deeper levels
    while stack.last().is_some_and(|&(d, _)| d > indent) {
        stack.pop();
    }
    if let Some(top) = stack.last_mut().filter(|t| t.0 == indent) {
        top.1 += 1;
        top.1
    } else {
        stack.push((indent, 1));
        1
    }
}

// `runs` entries are (byte_offset_into_para, run_ref). The run covers
// text from that offset up to the next entry's offset (or end of para).
fn emit_paragraph(
    out: &mut String,
    para: &str,
    runs: &[(usize, &AttributeRun)],
    para_style: &ParagraphStyle,
    list_number: usize,
    attachments: &HashMap<String, AttachmentContent>,
    link_resolver: Option<&LinkResolver<'_>>,
) {
    // Empty paragraphs get no styling — CRDT drift often puts blockquote/monospaced
    // style on blank lines between styled paragraphs.
    if para.trim().is_empty() {
        return;
    }

    let indent = para_style.indent as usize;
    let indent_str = "  ".repeat(indent);

    if para_style.block_quote {
        out.push_str(&indent_str);
        out.push_str("> ");
    } else if !indent_str.is_empty()
        && !matches!(
            para_style.style_type,
            StyleType::BulletList
                | StyleType::DashedList
                | StyleType::NumberedList
                | StyleType::Checklist
        )
    {
        out.push_str(&indent_str);
    }

    match para_style.style_type {
        StyleType::Title => out.push_str("# "),
        StyleType::Heading => out.push_str("## "),
        StyleType::Subheading => out.push_str("### "),
        StyleType::Monospaced => {
            out.push_str(&indent_str);
            out.push_str("```\n");
            out.push_str(para);
            out.push('\n');
            out.push_str(&indent_str);
            out.push_str("```");
            return;
        }
        StyleType::BulletList | StyleType::DashedList => {
            out.push_str(&indent_str);
            out.push_str("- ");
        }
        StyleType::NumberedList => {
            out.push_str(&indent_str);
            out.push_str(&format!("{}. ", list_number));
        }
        StyleType::Checklist => {
            out.push_str(&indent_str);
            let checked = para_style.checklist.as_ref().is_some_and(|c| c.done);
            out.push_str(if checked { "- [x] " } else { "- [ ] " });
        }
        StyleType::Body => {}
    }

    emit_inline(out, para, runs, attachments, link_resolver);
}

/// Pick the dominant paragraph style by weighted byte count.
/// If >50% of the paragraph has a non-Body style, use that style.
/// Otherwise default to Body. Handles CRDT drift where a Monospaced or
/// blockquote run bleeds a few chars into an adjacent Body paragraph.
fn dominant_para_style(para: &str, runs: &[(usize, &AttributeRun)]) -> ParagraphStyle {
    let para_len = para.len();
    if para_len == 0 || runs.is_empty() {
        return ParagraphStyle::default();
    }
    // Count bytes per style_type
    let mut style_bytes: Vec<(StyleType, bool, Option<&ChecklistInfo>, i32, usize)> = Vec::new();
    for (i, &(start, run)) in runs.iter().enumerate() {
        let end = if i + 1 < runs.len() {
            runs[i + 1].0.min(para_len)
        } else {
            para_len
        };
        let span = end.saturating_sub(start.min(para_len));
        let key = (
            run.style.style_type,
            run.style.block_quote,
            run.style.checklist.as_ref(),
            run.style.indent,
        );
        if let Some(entry) = style_bytes
            .iter_mut()
            .find(|e| e.0 == key.0 && e.1 == key.1 && e.3 == key.3)
        {
            entry.4 += span;
        } else {
            style_bytes.push((key.0, key.1, key.2, key.3, span));
        }
    }
    // Find the style with the most bytes
    let winner = style_bytes.iter().max_by_key(|e| e.4);
    match winner {
        Some(&(st, bq, cl, indent, _)) => ParagraphStyle {
            style_type: st,
            block_quote: bq,
            checklist: cl.cloned(),
            indent,
        },
        None => ParagraphStyle::default(),
    }
}

// `runs` are (byte_offset_into_para, run_ref) sorted ascending.
fn emit_inline(
    out: &mut String,
    para: &str,
    runs: &[(usize, &AttributeRun)],
    attachments: &HashMap<String, AttachmentContent>,
    link_resolver: Option<&LinkResolver<'_>>,
) {
    if runs.is_empty() {
        out.push_str(para);
        return;
    }

    // Per-span rendering: each run gets its own formatting markers.
    for (i, &(start, run)) in runs.iter().enumerate() {
        let end = if i + 1 < runs.len() {
            runs[i + 1].0
        } else {
            para.len()
        };
        let start = start.min(para.len());
        let end = end.min(para.len());
        let span = &para[start..end];
        if span.is_empty() {
            continue;
        }

        if let Some(ref att) = run.attachment {
            if let Some(content) = attachments.get(&att.identifier) {
                match content {
                    AttachmentContent::Table(table) => {
                        emit_table(out, table);
                    }
                    AttachmentContent::Image(uti, title) => {
                        let ext = attachment_extension(title.as_deref(), uti);
                        let alt = title.as_deref().unwrap_or("image");
                        out.push_str(&format!(
                            "![{}](/Attachments/{}{})",
                            alt, att.identifier, ext
                        ));
                    }
                    AttachmentContent::File(uti, title) => {
                        let ext = attachment_extension(title.as_deref(), uti);
                        let text = title.as_deref().unwrap_or("file");
                        out.push_str(&format!(
                            "[{}](/Attachments/{}{})",
                            text, att.identifier, ext
                        ));
                    }
                }
            } else {
                // Unresolved attachment — show type-specific placeholder
                let uti = att.type_uti.as_deref().unwrap_or("unknown");
                if uti == "com.apple.notes.table" {
                    out.push_str(&format!("[table](attachment:{})", att.identifier));
                } else if is_image_uti(uti) {
                    let ext = uti_to_extension(uti);
                    out.push_str(&format!("![image](/Attachments/{}{})", att.identifier, ext));
                } else {
                    let ext = uti_to_extension(uti);
                    out.push_str(&format!("[file](/Attachments/{}{})", att.identifier, ext));
                }
            }
            continue;
        }

        let mut prefix = String::new();
        let mut suffix = String::new();

        if let Some(ref url) = run.link {
            let resolved = link_resolver.and_then(|r| r(url));
            let url = resolved.as_deref().unwrap_or(url);
            prefix.push('[');
            suffix.push_str(&format!("]({})", url));
        }
        if run.font.bold && run.font.italic {
            prefix.push_str("***");
            suffix.insert_str(0, "***");
        } else if run.font.bold {
            prefix.push_str("**");
            suffix.insert_str(0, "**");
        } else if run.font.italic {
            prefix.push('*');
            suffix.insert(0, '*');
        }
        if run.strikethrough {
            prefix.push_str("~~");
            suffix.insert_str(0, "~~");
        }
        if run.underlined {
            prefix.push_str("<u>");
            suffix.insert_str(0, "</u>");
        }

        // If this span is a link, move leading/trailing whitespace outside the brackets
        if run.link.is_some() {
            let trimmed = span.trim();
            let leading = &span[..span.len() - span.trim_start().len()];
            let trailing = &span[span.trim_end().len()..];
            out.push_str(leading);
            out.push_str(&prefix);
            out.push_str(trimmed);
            out.push_str(&suffix);
            out.push_str(trailing);
        } else {
            out.push_str(&prefix);
            out.push_str(span);
            out.push_str(&suffix);
        }
    }
}

/// Render a table as a markdown pipe table.
fn emit_table(out: &mut String, table: &TableData) {
    if table.cells.is_empty() {
        return;
    }
    let num_cols = table.cols();
    if num_cols == 0 {
        return;
    }

    // Compute column widths
    let mut widths = vec![3usize; num_cols]; // minimum width of 3
    for row in &table.cells {
        for (j, cell) in row.iter().enumerate() {
            widths[j] = widths[j].max(cell.len());
        }
    }

    for (i, row) in table.cells.iter().enumerate() {
        out.push('|');
        for (j, cell) in row.iter().enumerate() {
            out.push(' ');
            out.push_str(cell);
            for _ in cell.len()..widths[j] {
                out.push(' ');
            }
            out.push_str(" |");
        }
        if i == 0 {
            // Emit separator after header
            out.push('\n');
            out.push('|');
            for w in &widths {
                out.push(' ');
                for _ in 0..*w {
                    out.push('-');
                }
                out.push_str(" |");
            }
        }
        if i + 1 < table.cells.len() {
            out.push('\n');
        }
    }
}

// ── Markdown → NoteDocument ───────────────────────────────

/// Result of parsing Markdown: the note document plus any tables to create.
pub struct ParsedNote {
    pub doc: NoteDocument,
    pub tables: Vec<TableData>,
}

/// Parse Markdown into a NoteDocument, extracting any pipe tables as pending attachments.
pub fn from_markdown(md: &str) -> Result<ParsedNote> {
    from_markdown_inner(md, None)
}

/// Parse Markdown with a link resolver that maps VFS paths back to original URLs.
/// e.g. `/Notes/Folder/Title.md` → `applenotes:note/UUID?ownerIdentifier=...`
pub fn from_markdown_with_context(
    md: &str,
    link_resolver: Option<&LinkResolver<'_>>,
) -> Result<ParsedNote> {
    from_markdown_inner(md, link_resolver)
}

fn from_markdown_inner(md: &str, link_resolver: Option<&LinkResolver<'_>>) -> Result<ParsedNote> {
    let mut text = String::new();
    let mut runs: Vec<AttributeRun> = Vec::new();
    let mut tables: Vec<TableData> = Vec::new();
    let mut first_line = true;

    // Collect lines and detect pipe tables
    let lines: Vec<&str> = md.lines().collect();
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];

        // Detect pipe table: line starts with '|'
        if line.trim_start().starts_with('|') && line.contains('|') {
            let table_start = i;
            let mut table_lines: Vec<&str> = Vec::new();
            while i < lines.len() && lines[i].trim_start().starts_with('|') {
                table_lines.push(lines[i]);
                i += 1;
            }

            if let Some(table) = parse_pipe_table(&table_lines) {
                // Generate a placeholder attachment ID for this table
                let att_id = uuid::Uuid::new_v4().to_string();

                // Insert U+FFFC as attachment placeholder
                text.push('\u{FFFC}');
                runs.push(AttributeRun {
                    length: 1,
                    style: ParagraphStyle::default(),
                    attachment: Some(AttachmentInfo {
                        identifier: att_id,
                        type_uti: Some("com.apple.notes.table".into()),
                    }),
                    ..Default::default()
                });
                text.push('\n');
                if let Some(last) = runs.last_mut() {
                    last.length += 1;
                }

                tables.push(table);
                // Skip any blank line after table
                if i < lines.len() && lines[i].trim().is_empty() {
                    // Will be handled by next iteration
                }
                continue;
            }
            // Not a valid table, fall through and process lines normally
            i = table_start;
        }

        let (content, style) = parse_line_prefix(line, first_line);
        first_line = false;
        let is_blank = content.is_empty();

        let parsed = parse_line_inline(content, &style, link_resolver);
        let mut pushed_any = false;
        for (span_text, mut run) in parsed {
            let len = span_text.chars().count();
            if len == 0 {
                continue;
            }
            run.length = len;
            text.push_str(&span_text);
            runs.push(run);
            pushed_any = true;
        }

        // Add newline. For blank lines we must NOT extend the previous run's
        // style — otherwise a blank line after `- [ ] Task 1` would grow the
        // Checklist run's length to 2 paragraphs, and Apple Notes would render
        // the trailing blank as a spurious empty checkbox. Blank lines always
        // start a fresh Body run that owns only their `\n`.
        if is_blank {
            text.push('\n');
            runs.push(AttributeRun {
                length: 1,
                style: ParagraphStyle::default(),
                ..Default::default()
            });
        } else if pushed_any {
            text.push('\n');
            if let Some(last) = runs.last_mut() {
                last.length += 1;
            }
        }
        i += 1;
    }

    // Ensure first paragraph is Title if not already
    if !runs.is_empty() && runs[0].style.style_type != StyleType::Title {
        runs[0].style.style_type = StyleType::Title;
    }

    Ok(ParsedNote {
        doc: NoteDocument { text, runs },
        tables,
    })
}

/// Parse a markdown pipe table from lines like `| a | b |`.
/// Skips the separator line (`| --- | --- |`).
fn parse_pipe_table(lines: &[&str]) -> Option<TableData> {
    if lines.len() < 2 {
        return None;
    }

    let mut rows: Vec<Vec<String>> = Vec::new();
    for line in lines {
        let trimmed = line.trim();
        // Skip separator lines (| --- | --- |)
        if is_separator_line(trimmed) {
            continue;
        }
        let cells: Vec<String> = trimmed
            .trim_matches('|')
            .split('|')
            .map(|c| c.trim().to_string())
            .collect();
        if !cells.is_empty() {
            rows.push(cells);
        }
    }

    if rows.is_empty() {
        return None;
    }

    // Normalize: ensure all rows have the same number of columns
    let max_cols = rows.iter().map(|r| r.len()).max().unwrap_or(0);
    for row in &mut rows {
        while row.len() < max_cols {
            row.push(String::new());
        }
    }

    Some(TableData { cells: rows })
}

fn is_separator_line(line: &str) -> bool {
    let stripped = line.trim().trim_matches('|');
    !stripped.is_empty()
        && stripped.split('|').all(|cell| {
            cell.trim()
                .chars()
                .all(|c| c == '-' || c == ':' || c == ' ')
        })
}

fn parse_line_prefix(line: &str, is_first: bool) -> (&str, ParagraphStyle) {
    let trimmed = line.trim_start();
    let indent = (line.len() - trimmed.len()) / 2;

    // Check for heading
    if let Some(rest) = trimmed.strip_prefix("### ") {
        return (
            rest,
            ParagraphStyle {
                style_type: if is_first {
                    StyleType::Title
                } else {
                    StyleType::Subheading
                },
                indent: indent as i32,
                ..Default::default()
            },
        );
    }
    if let Some(rest) = trimmed.strip_prefix("## ") {
        return (
            rest,
            ParagraphStyle {
                style_type: if is_first {
                    StyleType::Title
                } else {
                    StyleType::Heading
                },
                indent: indent as i32,
                ..Default::default()
            },
        );
    }
    if let Some(rest) = trimmed.strip_prefix("# ") {
        return (
            rest,
            ParagraphStyle {
                style_type: StyleType::Title,
                indent: indent as i32,
                ..Default::default()
            },
        );
    }

    // Checklist
    if let Some(rest) = trimmed
        .strip_prefix("- [x] ")
        .or_else(|| trimmed.strip_prefix("- [X] "))
    {
        return (
            rest,
            ParagraphStyle {
                style_type: StyleType::Checklist,
                indent: indent as i32,
                checklist: Some(ChecklistInfo { done: true }),
                ..Default::default()
            },
        );
    }
    if let Some(rest) = trimmed.strip_prefix("- [ ] ") {
        return (
            rest,
            ParagraphStyle {
                style_type: StyleType::Checklist,
                indent: indent as i32,
                checklist: Some(ChecklistInfo { done: false }),
                ..Default::default()
            },
        );
    }

    // Block quote
    if let Some(rest) = trimmed.strip_prefix("> ") {
        return (
            rest,
            ParagraphStyle {
                block_quote: true,
                indent: indent as i32,
                ..Default::default()
            },
        );
    }

    // Numbered list
    if let Some(dot_pos) = trimmed.find(". ") {
        if dot_pos <= 3 && trimmed[..dot_pos].chars().all(|c| c.is_ascii_digit()) {
            return (
                &trimmed[dot_pos + 2..],
                ParagraphStyle {
                    style_type: StyleType::NumberedList,
                    indent: indent as i32,
                    ..Default::default()
                },
            );
        }
    }

    // Bullet list
    if let Some(rest) = trimmed
        .strip_prefix("- ")
        .or_else(|| trimmed.strip_prefix("* "))
    {
        return (
            rest,
            ParagraphStyle {
                style_type: StyleType::BulletList,
                indent: indent as i32,
                ..Default::default()
            },
        );
    }

    // Indented code block (4 spaces)
    if let Some(rest) = trimmed.strip_prefix("    ") {
        if indent == 0 && line.starts_with("    ") {
            return (
                rest,
                ParagraphStyle {
                    style_type: StyleType::Monospaced,
                    ..Default::default()
                },
            );
        }
    }

    // Default body
    let style = if is_first {
        ParagraphStyle {
            style_type: StyleType::Title,
            indent: indent as i32,
            ..Default::default()
        }
    } else {
        ParagraphStyle {
            indent: indent as i32,
            ..Default::default()
        }
    };
    (trimmed, style)
}

fn parse_line_inline(
    content: &str,
    style: &ParagraphStyle,
    link_resolver: Option<&LinkResolver<'_>>,
) -> Vec<(String, AttributeRun)> {
    let mut results = Vec::new();
    let mut pos = 0;
    let mut current_text = String::new();
    let mut font = FontWeight::default();
    let mut strike = false;
    let mut underline = false;

    let flush = |results: &mut Vec<(String, AttributeRun)>,
                 text: &mut String,
                 font: FontWeight,
                 strike: bool,
                 underline: bool| {
        if !text.is_empty() {
            results.push((
                std::mem::take(text),
                make_inline_run(style, font, strike, underline, None),
            ));
        }
    };

    while pos < content.len() {
        let rest = &content[pos..];

        // Bold+Italic ***
        if rest.starts_with("***") {
            flush(&mut results, &mut current_text, font, strike, underline);
            font.bold = !font.bold;
            font.italic = !font.italic;
            pos += 3;
        } else if rest.starts_with("**") {
            flush(&mut results, &mut current_text, font, strike, underline);
            font.bold = !font.bold;
            pos += 2;
        } else if rest.starts_with('*') {
            flush(&mut results, &mut current_text, font, strike, underline);
            font.italic = !font.italic;
            pos += 1;
        } else if rest.starts_with("~~") {
            flush(&mut results, &mut current_text, font, strike, underline);
            strike = !strike;
            pos += 2;
        } else if rest.starts_with("<u>") {
            flush(&mut results, &mut current_text, font, strike, underline);
            underline = true;
            pos += 3;
        } else if rest.starts_with("</u>") {
            flush(&mut results, &mut current_text, font, strike, underline);
            underline = false;
            pos += 4;
        } else if rest.starts_with("![") {
            // Image syntax: ![alt](/Attachments/UUID.ext) → attachment run
            if let Some((alt, url, consumed)) = parse_link(&rest[1..]) {
                if let Some(att) = parse_attachment_url(&url, true) {
                    flush(&mut results, &mut current_text, font, strike, underline);
                    results.push(make_attachment_run(style, att));
                    pos += 1 + consumed; // +1 for the '!'
                } else {
                    // Not an attachment image, emit as-is
                    flush(&mut results, &mut current_text, font, strike, underline);
                    results.push((
                        format!("![{}]({})", alt, url),
                        make_inline_run(style, font, strike, underline, None),
                    ));
                    pos += 1 + consumed;
                }
            } else {
                current_text.push('!');
                pos += 1;
            }
        } else if rest.starts_with('[') {
            if let Some((link_text, url, consumed)) = parse_link(rest) {
                // Check if this is a file attachment link
                if let Some(att) = parse_attachment_url(&url, false) {
                    flush(&mut results, &mut current_text, font, strike, underline);
                    results.push(make_attachment_run(style, att));
                    pos += consumed;
                } else {
                    flush(&mut results, &mut current_text, font, strike, underline);
                    let resolved_url = link_resolver.and_then(|r| r(&url)).unwrap_or(url);
                    results.push((
                        link_text,
                        make_inline_run(style, font, strike, underline, Some(resolved_url)),
                    ));
                    pos += consumed;
                }
            } else {
                // Not a valid link, consume the '['
                current_text.push('[');
                pos += 1;
            }
        } else {
            // Consume one character (handle multi-byte)
            let ch = rest.chars().next().unwrap();
            current_text.push(ch);
            pos += ch.len_utf8();
        }
    }

    if !current_text.is_empty() {
        results.push((
            current_text,
            make_inline_run(style, font, strike, underline, None),
        ));
    }

    // If no results, emit an empty run with the paragraph style
    if results.is_empty() {
        results.push((
            String::new(),
            AttributeRun {
                length: 0,
                style: style.clone(),
                ..Default::default()
            },
        ));
    }

    results
}

fn make_inline_run(
    style: &ParagraphStyle,
    font: FontWeight,
    strikethrough: bool,
    underlined: bool,
    link: Option<String>,
) -> AttributeRun {
    AttributeRun {
        length: 0, // filled by caller
        style: style.clone(),
        font,
        strikethrough,
        underlined,
        link,
        attachment: None,
    }
}

fn make_attachment_run(style: &ParagraphStyle, att: AttachmentInfo) -> (String, AttributeRun) {
    (
        "\u{FFFC}".to_string(),
        AttributeRun {
            length: 0,
            style: style.clone(),
            attachment: Some(att),
            ..Default::default()
        },
    )
}

/// Parse `/Attachments/UUID.ext` URL into an AttachmentInfo.
/// `is_image` determines whether we infer image vs file UTI.
fn parse_attachment_url(url: &str, is_image: bool) -> Option<AttachmentInfo> {
    let filename = url.strip_prefix("/Attachments/")?;
    if filename.is_empty() {
        return None;
    }
    // Split into UUID and extension at the last '.'
    let (id, ext) = if let Some(dot) = filename.rfind('.') {
        (&filename[..dot], &filename[dot..])
    } else {
        (filename, "")
    };
    if id.is_empty() {
        return None;
    }
    let uti = if ext.is_empty() {
        if is_image {
            "public.png".to_string()
        } else {
            "unknown".to_string()
        }
    } else {
        extension_to_uti(ext)
    };
    Some(AttachmentInfo {
        identifier: id.to_string(),
        type_uti: Some(uti),
    })
}

fn parse_link(s: &str) -> Option<(String, String, usize)> {
    if !s.starts_with('[') {
        return None;
    }
    let close_bracket = s.find(']')?;
    let text = &s[1..close_bracket];
    let rest = &s[close_bracket + 1..];
    if !rest.starts_with('(') {
        return None;
    }
    let close_paren = rest.find(')')?;
    let url = &rest[1..close_paren];
    Some((
        text.to_string(),
        url.to_string(),
        close_bracket + 1 + close_paren + 1,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simple_body_to_markdown() {
        let doc = NoteDocument {
            text: "Hello\nWorld\n".to_string(),
            runs: vec![
                AttributeRun {
                    length: 6,
                    style: ParagraphStyle {
                        style_type: StyleType::Title,
                        ..Default::default()
                    },
                    ..Default::default()
                },
                AttributeRun {
                    length: 6,
                    style: ParagraphStyle::default(),
                    ..Default::default()
                },
            ],
        };
        let md = to_markdown(&doc);
        assert!(md.contains("# Hello"));
        assert!(md.contains("World"));
    }

    #[test]
    fn checklist_to_markdown() {
        let doc = NoteDocument {
            text: "Title\nDone\nTodo\n".to_string(),
            runs: vec![
                AttributeRun {
                    length: 6,
                    style: ParagraphStyle {
                        style_type: StyleType::Title,
                        ..Default::default()
                    },
                    ..Default::default()
                },
                AttributeRun {
                    length: 5,
                    style: ParagraphStyle {
                        style_type: StyleType::Checklist,
                        checklist: Some(ChecklistInfo { done: true }),
                        ..Default::default()
                    },
                    ..Default::default()
                },
                AttributeRun {
                    length: 5,
                    style: ParagraphStyle {
                        style_type: StyleType::Checklist,
                        checklist: Some(ChecklistInfo { done: false }),
                        ..Default::default()
                    },
                    ..Default::default()
                },
            ],
        };
        let md = to_markdown(&doc);
        assert!(md.contains("- [x] Done"));
        assert!(md.contains("- [ ] Todo"));
    }

    #[test]
    fn markdown_parse_headings() {
        let md = "# My Title\n## Section\n### Sub\nBody text\n";
        let parsed = from_markdown(md).unwrap();
        assert!(parsed.doc.text.contains("My Title"));
        assert_eq!(parsed.doc.runs[0].style.style_type, StyleType::Title);
    }

    #[test]
    fn markdown_parse_checklist() {
        let md = "# Title\n- [x] Done\n- [ ] Todo\n";
        let parsed = from_markdown(md).unwrap();
        // Find the checklist runs
        let checklist_runs: Vec<_> = parsed
            .doc
            .runs
            .iter()
            .filter(|r| r.style.style_type == StyleType::Checklist)
            .collect();
        assert_eq!(checklist_runs.len(), 2);
        assert!(checklist_runs[0].style.checklist.as_ref().unwrap().done);
        assert!(!checklist_runs[1].style.checklist.as_ref().unwrap().done);
    }

    #[test]
    fn fragmented_bold_runs_are_merged() {
        // Apple Notes CRDT often fragments bold across word boundaries.
        // e.g. "git bise" (not bold) + "ct does stuff" (bold) should merge to
        // the whole line being bold, not "git bise**ct does stuff**".
        let doc = NoteDocument {
            text: "git bisect does stuff\n".to_string(),
            runs: vec![
                // Fragmented: bold starts mid-word at "ct"
                AttributeRun {
                    length: 10, // "git bisect" — but this is NOT bold
                    ..Default::default()
                },
                AttributeRun {
                    length: 11, // " does stuff\n" — bold
                    font: FontWeight {
                        bold: true,
                        italic: false,
                    },
                    ..Default::default()
                },
            ],
        };
        let md = to_markdown(&doc);
        // Should NOT produce "git bise**ct does stuff**" (mid-word marker)
        assert!(
            !md.contains("bise**ct"),
            "bold marker landed mid-word: {md}"
        );
    }

    #[test]
    fn same_format_runs_merge_across_body() {
        // Two adjacent body-style runs with same bold formatting should merge
        let doc = NoteDocument {
            text: "hello world\n".to_string(),
            runs: vec![
                AttributeRun {
                    length: 5, // "hello"
                    font: FontWeight {
                        bold: true,
                        italic: false,
                    },
                    ..Default::default()
                },
                AttributeRun {
                    length: 7, // " world\n"
                    font: FontWeight {
                        bold: true,
                        italic: false,
                    },
                    ..Default::default()
                },
            ],
        };
        let md = to_markdown(&doc);
        assert!(md.contains("**hello world**"), "runs should merge: {md}");
    }

    #[test]
    fn markdown_parse_bold_italic() {
        let md = "# Title\n**bold** and *italic*\n";
        let parsed = from_markdown(md).unwrap();
        let bold_run = parsed.doc.runs.iter().find(|r| r.font.bold);
        assert!(bold_run.is_some());
        let italic_run = parsed.doc.runs.iter().find(|r| r.font.italic);
        assert!(italic_run.is_some());
    }

    #[test]
    fn parse_pipe_table_from_markdown() {
        let md = "# Title\n| A | B |\n| --- | --- |\n| 1 | 2 |\n| 3 | 4 |\n";
        let parsed = from_markdown(md).unwrap();
        assert_eq!(parsed.tables.len(), 1);
        let t = &parsed.tables[0];
        assert_eq!(t.rows(), 3);
        assert_eq!(t.cols(), 2);
        assert_eq!(t.cells[0][0], "A");
        assert_eq!(t.cells[0][1], "B");
        assert_eq!(t.cells[1][0], "1");
        assert_eq!(t.cells[2][1], "4");

        // Doc should have U+FFFC placeholder
        assert!(parsed.doc.text.contains('\u{FFFC}'));
        // Should have a table attachment reference
        let att = parsed.doc.runs.iter().find(|r| r.attachment.is_some());
        assert!(att.is_some());
        let att_info = att.unwrap().attachment.as_ref().unwrap();
        assert_eq!(att_info.type_uti.as_deref(), Some("com.apple.notes.table"));
    }

    #[test]
    fn render_table_attachment_as_pipe_table() {
        let table = TableData {
            cells: vec![
                vec!["Name".into(), "Value".into()],
                vec!["foo".into(), "42".into()],
            ],
        };
        let mut attachments = HashMap::new();
        attachments.insert("test-uuid".to_string(), AttachmentContent::Table(table));

        let doc = NoteDocument {
            text: "Title\n\u{FFFC}\n".to_string(),
            runs: vec![
                AttributeRun {
                    length: 6,
                    style: ParagraphStyle {
                        style_type: StyleType::Title,
                        ..Default::default()
                    },
                    ..Default::default()
                },
                AttributeRun {
                    length: 1,
                    attachment: Some(AttachmentInfo {
                        identifier: "test-uuid".into(),
                        type_uti: Some("com.apple.notes.table".into()),
                    }),
                    ..Default::default()
                },
                AttributeRun {
                    length: 1,
                    ..Default::default()
                },
            ],
        };

        let md = to_markdown_with_attachments(
            &doc,
            &attachments,
            None::<&dyn Fn(&str) -> Option<String>>,
        );
        assert!(md.contains("| Name"), "table header missing: {md}");
        assert!(md.contains("| foo"), "table body missing: {md}");
        assert!(md.contains("| ---"), "separator missing: {md}");
    }

    #[test]
    fn render_image_attachment() {
        let doc = NoteDocument {
            text: "Title\n\u{FFFC}\n".to_string(),
            runs: vec![
                AttributeRun {
                    length: 6,
                    style: ParagraphStyle {
                        style_type: StyleType::Title,
                        ..Default::default()
                    },
                    ..Default::default()
                },
                AttributeRun {
                    length: 1,
                    attachment: Some(AttachmentInfo {
                        identifier: "img-uuid".into(),
                        type_uti: Some("public.png".into()),
                    }),
                    ..Default::default()
                },
                AttributeRun {
                    length: 1,
                    ..Default::default()
                },
            ],
        };

        let md = to_markdown(&doc);
        assert!(
            md.contains("![image](/Attachments/img-uuid.png)"),
            "image ref missing: {md}"
        );
    }

    #[test]
    fn roundtrip_title_body_checklist_is_stable() {
        // Regression for the "note duplicated in Apple Notes after edit" bug.
        // Start from what `to_markdown` would produce for a note with title
        // "New Note", a body paragraph "Testing?", and one unchecked task —
        // round-trip it through parse → render → parse and make sure the
        // resulting `doc.text` stays the same (no growing "New Note\nNew Note").
        let original_md = "# New Note\n\nTesting?\n\n- [ ] Task 1\n";
        let parsed = from_markdown(original_md).unwrap();
        let doc1 = parsed.doc.clone();
        let rendered = to_markdown(&doc1);
        let parsed2 = from_markdown(&rendered).unwrap();
        let doc2 = parsed2.doc;

        // Title should be "New Note", not "New Note\nNew Note".
        let title_line = doc2.text.lines().next().unwrap_or_default();
        assert_eq!(
            title_line, "New Note",
            "title drifted on round-trip; text = {:?}",
            doc2.text
        );
        // The word "New Note" must appear exactly once.
        assert_eq!(
            doc2.text.matches("New Note").count(),
            1,
            "title duplicated on round-trip; text = {:?}",
            doc2.text
        );
        // Exactly one checklist paragraph should survive.
        let checklists = doc2
            .runs
            .iter()
            .filter(|r| r.style.style_type == StyleType::Checklist)
            .count();
        assert_eq!(
            checklists, 1,
            "checklist count drift; runs = {:#?}",
            doc2.runs
        );
    }

    #[test]
    fn roundtrip_proto_encode_decode_is_stable() {
        // Full cycle: markdown → doc → proto bytes → doc → markdown.
        // Verifies that `encode_note_body` + `decode_note_body` preserves text
        // and paragraph styles (modulo the CRDT metadata we regenerate each
        // time).
        let original_md = "# New Note\n\nTesting?\n\n- [ ] Task 1\n";
        let parsed = from_markdown(original_md).unwrap();
        let b64 = crate::notes::proto::encode_note_body(&parsed.doc).unwrap();
        let decoded = crate::notes::proto::decode_note_body(&b64).unwrap();

        assert_eq!(
            decoded.text, parsed.doc.text,
            "proto roundtrip lost text content"
        );
        assert_eq!(
            decoded.text.matches("New Note").count(),
            1,
            "text duplicated on proto roundtrip: {:?}",
            decoded.text
        );

        let re_rendered = to_markdown(&decoded);
        assert_eq!(
            re_rendered.matches("New Note").count(),
            1,
            "title duplicated on md→proto→md roundtrip: {:?}",
            re_rendered
        );
    }

    #[test]
    fn blank_line_after_checklist_does_not_extend_checklist_run() {
        // Regression: a trailing blank line was being folded into the
        // previous Checklist run, so Apple Notes rendered a spurious empty
        // checkbox under the last real task. After the fix the trailing \n
        // must belong to a Body-style run instead.
        let md = "# Title\n- [ ] Task 1\n\n";
        let parsed = from_markdown(md).unwrap();
        let checklists: Vec<_> = parsed
            .doc
            .runs
            .iter()
            .filter(|r| r.style.style_type == StyleType::Checklist)
            .collect();
        assert_eq!(checklists.len(), 1, "should have exactly one checklist run");
        // "- [ ] Task 1" → "Task 1\n" = 7 chars; the run must NOT include the
        // trailing blank line.
        assert_eq!(
            checklists[0].length, 7,
            "checklist run grew across a blank line: runs = {:#?}",
            parsed.doc.runs
        );
    }

    #[test]
    fn blank_line_after_bullet_does_not_extend_bullet_run() {
        let md = "# Title\n- Item\n\nBody\n";
        let parsed = from_markdown(md).unwrap();
        let bullets: Vec<_> = parsed
            .doc
            .runs
            .iter()
            .filter(|r| r.style.style_type == StyleType::BulletList)
            .collect();
        assert_eq!(bullets.len(), 1);
        assert_eq!(bullets[0].length, 5, "`Item\\n` = 5 chars");
    }

    #[test]
    fn roundtrip_two_blank_lines_between_title_and_body() {
        // The edit VFS file on disk has TWO blank lines between `# New Note`
        // and the body (visible in the nvim screenshot the user posted).
        // Make sure that shape also round-trips stably.
        let md = "# New Note\n\n\nTesting?\n\n- [ ] Task 1\n";
        let parsed = from_markdown(md).unwrap();
        let rendered = to_markdown(&parsed.doc);
        assert_eq!(
            rendered.matches("New Note").count(),
            1,
            "title duplicated; rendered = {:?}",
            rendered
        );
        assert_eq!(
            rendered.matches("Testing?").count(),
            1,
            "body duplicated; rendered = {:?}",
            rendered
        );
    }

    #[test]
    fn mixed_formatting_not_collapsed() {
        // A paragraph with 60% bold should NOT have the whole thing bolded.
        let doc = NoteDocument {
            text: "Title\nHello world is amazing foo\n".to_string(),
            runs: vec![
                AttributeRun {
                    length: 6, // "Title\n"
                    style: ParagraphStyle {
                        style_type: StyleType::Title,
                        ..Default::default()
                    },
                    ..Default::default()
                },
                AttributeRun {
                    length: 6, // "Hello "
                    ..Default::default()
                },
                AttributeRun {
                    length: 16, // "world is amazing"
                    font: FontWeight {
                        bold: true,
                        italic: false,
                    },
                    ..Default::default()
                },
                AttributeRun {
                    length: 5, // " foo\n"
                    ..Default::default()
                },
            ],
        };
        let md = to_markdown(&doc);
        assert!(
            md.contains("**world is amazing**"),
            "bold span missing: {md}"
        );
        assert!(
            !md.contains("**Hello"),
            "non-bold text incorrectly bolded: {md}"
        );
    }
}
