//! Title ↔ filename mapping (SPEC: slash → U+2215, collisions, 255-byte cap).

use std::collections::HashSet;

const DIV_SLASH: char = '\u{2215}'; // ∕

pub use icloud_api::notes::markdown::title_to_filename_stem;

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
