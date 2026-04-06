use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct Reminder {
    pub id: String,
    pub title: String,
    pub completed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completion_date: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub due: Option<String>,
    pub priority: i32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub list_ref: Option<String>,
    pub list_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_ref: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modified_ts: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ReminderList {
    pub id: String,
    pub name: String,
}

pub fn priority_label(p: i32) -> &'static str {
    match p {
        1 => "high",
        5 => "medium",
        9 => "low",
        _ => "",
    }
}

pub fn parse_priority(s: &str) -> Option<i32> {
    match s.to_ascii_lowercase().as_str() {
        "high" | "h" => Some(1),
        "medium" | "m" => Some(5),
        "low" | "l" => Some(9),
        "none" | "" => Some(0),
        _ => None,
    }
}
