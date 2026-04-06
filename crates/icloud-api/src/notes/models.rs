//! Domain types for Apple Notes: document model, rich text attributes, folders.

use serde::Serialize;

// ── Style enums ───────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StyleType {
    #[default]
    Body,
    Title,
    Heading,
    Subheading,
    Monospaced,
    BulletList,
    DashedList,
    NumberedList,
    Checklist,
}

impl From<i64> for StyleType {
    fn from(v: i64) -> Self {
        match v {
            0 => Self::Title,
            1 => Self::Heading,
            2 => Self::Subheading,
            4 => Self::Monospaced,
            100 => Self::BulletList,
            101 => Self::DashedList,
            102 => Self::NumberedList,
            103 => Self::Checklist,
            _ => Self::Body,
        }
    }
}

impl From<StyleType> for i64 {
    fn from(s: StyleType) -> Self {
        match s {
            StyleType::Body => -1,
            StyleType::Title => 0,
            StyleType::Heading => 1,
            StyleType::Subheading => 2,
            StyleType::Monospaced => 4,
            StyleType::BulletList => 100,
            StyleType::DashedList => 101,
            StyleType::NumberedList => 102,
            StyleType::Checklist => 103,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FontWeight {
    pub bold: bool,
    pub italic: bool,
}

impl From<i64> for FontWeight {
    fn from(v: i64) -> Self {
        Self {
            bold: v & 1 != 0,
            italic: v & 2 != 0,
        }
    }
}

impl From<FontWeight> for i64 {
    fn from(f: FontWeight) -> Self {
        (f.bold as i64) | ((f.italic as i64) << 1)
    }
}

// ── Document model ────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct NoteDocument {
    pub text: String,
    pub runs: Vec<AttributeRun>,
}

#[derive(Debug, Clone, Default)]
pub struct AttributeRun {
    pub length: usize,
    pub style: ParagraphStyle,
    pub font: FontWeight,
    pub underlined: bool,
    pub strikethrough: bool,
    pub link: Option<String>,
    pub attachment: Option<AttachmentInfo>,
}

#[derive(Debug, Clone, Default)]
pub struct ParagraphStyle {
    pub style_type: StyleType,
    pub indent: i32,
    pub checklist: Option<ChecklistInfo>,
    pub block_quote: bool,
}

#[derive(Debug, Clone)]
pub struct ChecklistInfo {
    pub done: bool,
}

#[derive(Debug, Clone)]
pub struct AttachmentInfo {
    pub identifier: String,
    pub type_uti: Option<String>,
}

// ── Public-facing types (for CLI output) ──────────────────

#[derive(Debug, Clone, Serialize)]
pub struct Note {
    pub id: String,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snippet: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub folder_id: Option<String>,
    pub folder_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modified: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct NoteFolder {
    pub id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<String>,
}
