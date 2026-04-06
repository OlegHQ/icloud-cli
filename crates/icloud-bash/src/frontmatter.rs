//! YAML frontmatter for note and reminder `.md` VFS files.

use std::borrow::Cow;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NoteFrontmatter {
    pub id: String,
    pub folder: String,
    pub modified: String,
}

fn render_frontmatter(fm: &impl Serialize, body: &str) -> String {
    let yaml = serde_yaml::to_string(fm).unwrap_or_default();
    let mut out = String::with_capacity(4 + yaml.len() + 4 + body.len());
    out.push_str("---\n");
    out.push_str(&yaml);
    out.push_str("---\n");
    out.push_str(body);
    out
}

pub fn render_note(fm: &NoteFrontmatter, body: &str) -> String {
    render_frontmatter(fm, body)
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ReminderFrontmatter {
    pub id: String,
    pub list: String,
    pub completed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub due: Option<String>,
    pub priority: String,
    #[serde(default)]
    pub notes: String,
}

pub fn render_reminder(fm: &ReminderFrontmatter, body: &str) -> String {
    render_frontmatter(fm, body)
}

/// Split optional YAML frontmatter; returns (body after fm, optional yaml text).
pub fn strip_frontmatter(input: &str) -> (Cow<'_, str>, Option<String>) {
    let t = input.trim_start();
    if !t.starts_with("---") {
        return (Cow::Borrowed(input), None);
    }
    let after_open = t[3..]
        .strip_prefix('\n')
        .or_else(|| t[3..].strip_prefix("\r\n"))
        .unwrap_or(&t[3..]);
    for delim in ["\n---\n", "\r\n---\n", "\n---\r\n", "\r\n---\r\n"] {
        if let Some(i) = after_open.find(delim) {
            let fm = after_open[..i].trim();
            let body = &after_open[i + delim.len()..];
            return (Cow::Owned(body.to_string()), Some(fm.to_string()));
        }
    }
    (Cow::Borrowed(input), None)
}

#[derive(Debug, Default)]
pub struct ReminderWriteFields {
    pub title: String,
    pub completed: Option<bool>,
    pub due: Option<Option<String>>,
    pub clear_due: bool,
    pub priority: Option<String>,
    pub notes: Option<String>,
}

pub fn parse_reminder_write_input(input: &str) -> Result<(ReminderWriteFields, String), String> {
    let (body, fm_text) = strip_frontmatter(input);
    let mut fields = ReminderWriteFields {
        title: body.trim().to_string(),
        ..Default::default()
    };
    let Some(yaml) = fm_text else {
        return Ok((fields, body.into_owned()));
    };
    let v: serde_yaml::Value = serde_yaml::from_str(&yaml).map_err(|e| e.to_string())?;
    if let Some(c) = v.get("completed").and_then(|x| x.as_bool()) {
        fields.completed = Some(c);
    }
    if let Some(d) = v.get("due") {
        if d.is_null() {
            fields.clear_due = true;
        } else if let Some(s) = d.as_str() {
            fields.due = Some(Some(s.to_string()));
        }
    }
    if let Some(p) = v.get("priority").and_then(|x| x.as_str()) {
        fields.priority = Some(p.to_string());
    }
    if let Some(n) = v.get("notes") {
        if let Some(s) = n.as_str() {
            fields.notes = Some(s.to_string());
        }
    }
    Ok((fields, body.into_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_then_body() {
        let s = "---\nid: x\n---\n# Hi\n\nBody\n";
        let (b, fm) = strip_frontmatter(s);
        assert!(fm.is_some());
        assert!(b.contains("Body"));
    }
}
