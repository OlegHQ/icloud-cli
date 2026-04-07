//! Integration tests for shared VFS helpers and `bashbox` wiring.
//! Detailed behavior is covered by unit tests in each module; CloudKit-backed
//! `ICloudFs` is exercised manually or via `icloud bash` with a real session.

use std::collections::HashSet;
use std::sync::Arc;

use icloud_bash::frontmatter::{render_note, strip_frontmatter, NoteFrontmatter};
use icloud_bash::pathmap::{classify, normalize_vpath, VfsTarget};
use icloud_bash::sanitize::{disambiguate_filename, filename_to_title, title_to_filename_stem};
use bashbox::bash::BashOptions;
use bashbox::{Bash, InMemoryFs};

#[tokio::test(flavor = "multi_thread")]
async fn bash_in_memory_runs_simple_command() {
    let fs = Arc::new(InMemoryFs::new());
    let mut bash = Bash::new(BashOptions {
        fs: Some(fs),
        cwd: Some("/".to_string()),
        env: None,
        limits: None,
    })
    .await;
    let res = bash.exec("echo ok", None).await;
    assert_eq!(res.exit_code, 0, "stderr={}", res.stderr);
    assert!(res.stdout.contains("ok"), "stdout={}", res.stdout);
}

#[test]
fn normalize_collapses_dot_segments() {
    assert_eq!(
        normalize_vpath("/Notes/./Inbox/../Inbox/note.md"),
        "/Notes/Inbox/note.md"
    );
}

#[test]
fn classify_notes_file_path() {
    let p = normalize_vpath("/Notes/Inbox/Hello.md");
    match classify(&p).unwrap() {
        VfsTarget::NotesFile {
            folder_name,
            filename,
        } => {
            assert_eq!(folder_name, "Inbox");
            assert_eq!(filename, "Hello.md");
        }
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn strip_frontmatter_roundtrips_with_render_note() {
    let fm = NoteFrontmatter {
        id: "id-1".into(),
        folder: "F".into(),
        modified: "2024-01-01".into(),
    };
    let rendered = render_note(&fm, "body text");
    let (body, _) = strip_frontmatter(&rendered);
    assert_eq!(body.trim(), "body text");
}

#[test]
fn filename_stem_collision_matches_disambiguation() {
    let stem = title_to_filename_stem("Dup");
    let existing: HashSet<String> = [format!("{stem}.md").to_ascii_lowercase()]
        .into_iter()
        .collect();
    let second = disambiguate_filename(&stem, &existing);
    assert!(second.contains("(2)"), "got {second}");
    assert_eq!(filename_to_title(&second), "Dup (2)");
}

#[tokio::test]
#[ignore = "Multi-process redb stress belongs in icloud-api; run when tooling allows"]
async fn ignored_placeholder_multi_process_redb() {}
