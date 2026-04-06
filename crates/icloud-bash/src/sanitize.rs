//! Title ↔ filename mapping (SPEC: slash → U+2215, collisions, 255-byte cap).

use std::collections::HashSet;

const DIV_SLASH: char = '\u{2215}'; // ∕
const MAX_BYTES: usize = 255;

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
    collapse_spaces_trim(&mut s);
    if s.is_empty() {
        return "Untitled".to_string();
    }
    truncate_chars_bytes(&mut s, MAX_BYTES);
    s
}

/// Build a unique `.md` filename inside a folder, appending ` (2)`, ` (3)`, … on collision.
///
/// `existing` must contain **lowercased** filenames for O(1) collision checks.
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

fn collapse_spaces_trim(s: &mut String) {
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
    *s = t;
}

fn truncate_chars_bytes(s: &mut String, max_bytes: usize) {
    if s.len() <= max_bytes {
        return;
    }
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    s.truncate(end);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slash_becomes_division_slash() {
        assert_eq!(title_to_filename_stem("a/b"), format!("a{}b", DIV_SLASH));
    }

    #[test]
    fn null_removed() {
        assert_eq!(title_to_filename_stem("a\0b"), "a b");
    }

    #[test]
    fn empty_is_untitled() {
        assert_eq!(title_to_filename_stem("   "), "Untitled");
    }

    #[test]
    fn collision_suffix() {
        let ex: HashSet<String> = ["hi.md".to_string()].into_iter().collect();
        assert_eq!(disambiguate_filename("Hi", &ex), "Hi (2).md");
        let ex2: HashSet<String> = ["hi.md".to_string(), "hi (2).md".to_string()]
            .into_iter()
            .collect();
        assert_eq!(disambiguate_filename("Hi", &ex2), "Hi (3).md");
    }

    #[test]
    fn collision_case_insensitive() {
        let ex: HashSet<String> = ["hello.md".to_string()].into_iter().collect();
        // "HELLO" stem collides with lowercased "hello.md"
        assert_eq!(disambiguate_filename("HELLO", &ex), "HELLO (2).md");
    }

    #[test]
    fn no_collision_empty_set() {
        let ex: HashSet<String> = HashSet::new();
        assert_eq!(disambiguate_filename("Test", &ex), "Test.md");
    }

    #[test]
    fn roundtrip_div_slash() {
        let t = "foo/bar";
        let stem = title_to_filename_stem(t);
        let name = format!("{stem}.md");
        assert_eq!(filename_to_title(&name), t);
    }
}
