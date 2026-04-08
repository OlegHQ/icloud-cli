//! Virtual path classification and normalization.

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VfsTarget {
    Root,
    /// `/Notes` listing
    NotesRoot,
    /// `/Notes/<folder>/` directory
    NotesFolder {
        folder_name: String,
    },
    /// `/Notes/<folder>/<file>.md`
    NotesFile {
        folder_name: String,
        filename: String,
    },
    RemindersRoot,
    RemindersList {
        list_name: String,
    },
    RemindersFile {
        list_name: String,
        filename: String,
    },
    HideMyEmailRoot,
    HideMyEmailAliases,
    /// `/Attachments` listing
    AttachmentsRoot,
    /// `/Attachments/<file>`
    AttachmentsFile {
        filename: String,
    },
    /// Paths delegated to the passthrough (in-memory) layer — not under iCloud prefixes.
    Passthrough,
}

/// Normalize a path: leading slash, trim trailing `/` (except root), resolve `.` / `..`.
pub fn normalize_vpath(path: &str) -> String {
    if path.is_empty() || path == "/" {
        return "/".to_string();
    }
    // Fast path: already normalized (starts with `/`, no trailing `/`, no `//`, no `/.`)
    if path.len() > 1
        && path.starts_with('/')
        && !path.ends_with('/')
        && !path.contains("//")
        && !path.contains("/.")
    {
        return path.to_string();
    }
    let mut p = path.to_string();
    if !p.starts_with('/') {
        p = format!("/{p}");
    }
    if p.len() > 1 && p.ends_with('/') {
        p.pop();
    }
    let parts: Vec<&str> = p
        .split('/')
        .filter(|s| !s.is_empty() && *s != ".")
        .collect();
    let mut stack: Vec<&str> = Vec::new();
    for part in parts {
        if part == ".." {
            stack.pop();
        } else {
            stack.push(part);
        }
    }
    if stack.is_empty() {
        "/".to_string()
    } else {
        format!("/{}", stack.join("/"))
    }
}

/// Classify a **normalized** virtual path into a [`VfsTarget`].
///
/// Callers must pass an already-normalized path (via [`normalize_vpath`]).
/// This avoids redundant normalization on every classify call.
#[allow(clippy::result_unit_err)] // Sentinel `()` for invalid paths; callers map to [`FsError`].
pub fn classify(path: &str) -> Result<VfsTarget, ()> {
    // Trust that the caller has already normalized the path.
    let n = path;
    if n == "/" {
        return Ok(VfsTarget::Root);
    }
    let parts: Vec<&str> = n.trim_start_matches('/').split('/').collect();
    match parts.first().copied() {
        None => Ok(VfsTarget::Root),
        Some("Notes") => match parts.len() {
            1 => Ok(VfsTarget::NotesRoot),
            2 => Ok(VfsTarget::NotesFolder {
                folder_name: parts[1].to_string(),
            }),
            3 => {
                if !parts[2].ends_with(".md") {
                    return Err(());
                }
                Ok(VfsTarget::NotesFile {
                    folder_name: parts[1].to_string(),
                    filename: parts[2].to_string(),
                })
            }
            _ => Err(()),
        },
        Some("Reminders") => match parts.len() {
            1 => Ok(VfsTarget::RemindersRoot),
            2 => Ok(VfsTarget::RemindersList {
                list_name: parts[1].to_string(),
            }),
            3 => {
                if !parts[2].ends_with(".md") {
                    return Err(());
                }
                Ok(VfsTarget::RemindersFile {
                    list_name: parts[1].to_string(),
                    filename: parts[2].to_string(),
                })
            }
            _ => Err(()),
        },
        Some("HideMyEmail") => match parts.len() {
            1 => Ok(VfsTarget::HideMyEmailRoot),
            2 if parts[1] == "aliases.json" => Ok(VfsTarget::HideMyEmailAliases),
            _ => Err(()),
        },
        Some("Attachments") => match parts.len() {
            1 => Ok(VfsTarget::AttachmentsRoot),
            2 => Ok(VfsTarget::AttachmentsFile {
                filename: parts[1].to_string(),
            }),
            _ => Err(()),
        },
        _ => Ok(VfsTarget::Passthrough),
    }
}

/// Check if a **normalized** path falls under an iCloud-managed prefix.
///
/// Callers must pass an already-normalized path (via [`normalize_vpath`]).
pub fn is_icloud_prefix(path: &str) -> bool {
    path == "/"
        || path.starts_with("/Notes")
        || path.starts_with("/Reminders")
        || path.starts_with("/HideMyEmail")
        || path.starts_with("/Attachments")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_dots() {
        assert_eq!(
            normalize_vpath("/Notes/./Work/../Notes/a.md"),
            "/Notes/Notes/a.md"
        );
    }

    #[test]
    fn normalize_fast_path() {
        // Already-normalized path should be returned as-is (fast path).
        assert_eq!(
            normalize_vpath("/Notes/Work/Hello.md"),
            "/Notes/Work/Hello.md"
        );
    }

    #[test]
    fn normalize_trailing_slash() {
        assert_eq!(normalize_vpath("/Notes/Work/"), "/Notes/Work");
    }

    #[test]
    fn classify_notes_file() {
        // classify now expects normalized input
        let path = normalize_vpath("/Notes/Work/Hello.md");
        match classify(&path).unwrap() {
            VfsTarget::NotesFile {
                folder_name,
                filename,
            } => {
                assert_eq!(folder_name, "Work");
                assert_eq!(filename, "Hello.md");
            }
            _ => panic!(),
        }
    }

    #[test]
    fn classify_root() {
        assert_eq!(classify("/").unwrap(), VfsTarget::Root);
    }

    #[test]
    fn classify_passthrough() {
        assert_eq!(classify("/tmp/foo").unwrap(), VfsTarget::Passthrough);
    }

    #[test]
    fn is_icloud_prefix_notes() {
        assert!(is_icloud_prefix("/Notes"));
        assert!(is_icloud_prefix("/Notes/Work"));
        assert!(is_icloud_prefix("/"));
        assert!(!is_icloud_prefix("/tmp"));
    }
}
