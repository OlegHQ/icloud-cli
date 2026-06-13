# Progress

Handoff log for cross-session work. New sessions should skim this before planning larger changes.

## 2026-06-13 - Reminders VFS body-only write fix release

Fixed the release blocker found in the installed CLI re-smoke: existing
Reminder files now treat a non-empty Markdown body as the desired reminder
title and call `edit_reminder()` when that title differs from the cached title,
even if no writable YAML frontmatter fields changed.

### Changed

- Bumped the workspace version to `0.1.1` for the fix release.
- Added Reminders VFS write-planning tests for:
  - body-only writes requesting a title update
  - same-title body writes avoiding unnecessary CloudKit edits
  - frontmatter-only writes updating metadata without forcing filename-derived
    title changes
- Installed the fixed binary with
  `cargo install --path crates/icloud-cli --root /home/snowbear/.local --force --locked`.

### Verified

- `cargo fmt --all -- --check`
- `cargo test -p icloud-bash reminder_ --locked`
- `cargo build -p icloud-cli --locked`
- `cargo test --workspace --locked`
- `cargo clippy --workspace --all-targets --locked -- -D warnings`
- Live disposable smoke against `target/debug/icloud`:
  - body-only `icloud cp` renamed an existing Reminder
  - `icloud bash -c 'echo new-title > /Reminders/<list>/<old>.md'` renamed an
    existing Reminder
  - frontmatter notes/priority update still worked and read back through VFS
- Installed binary smoke:
  - `icloud version` prints `icloud 0.1.1`
  - body-only `icloud cp` renamed an existing Reminder

### Release

- Commit and push this state, then push tag `v0.1.1` to trigger the release
  workflow.
- `cargo install --locked` still warns that locked `fastrand 2.4.0` is yanked;
  the install succeeds, but dependency refresh remains a follow-up before a
  broader public release.

## 2026-06-13 - Installed CLI re-smoke after release fixes

Re-tested the installed binary at `/home/snowbear/.local/bin/icloud`
(`icloud 0.1.0`) against the already-authenticated account with disposable
`codex-*` Notes folders, Notes, Reminders lists, and Reminders.

### Verified

- Local gates: `cargo test --workspace --locked`,
  `cargo fmt --all -- --check`, and
  `cargo clippy --workspace --all-targets -- -D warnings` all passed.
- Session validation: `icloud --no-input --max-age 0 --quiet whoami`
  returned `ok`.
- Root VFS and `/tmp`: `icloud bash -c 'ls /'` showed
  `HideMyEmail, Notes, Reminders, tmp`; escaped-space redirect/read under
  `/tmp` worked.
- Notes CLI/VFS: folder create, Markdown note create/get/update, Markdown title
  rename through `icloud cp`, VFS read, escaped-space VFS write, host
  `cp` -> iCloud -> host roundtrip, and Notes search index all worked.
- Reminders CLI/VFS: list create, add, edit, list, read existing reminder
  through VFS, complete, and recursive list cleanup all worked.
- HME read-only smoke: `icloud hme list --quiet` returned a count and
  `/HideMyEmail/aliases.json` exists in the VFS without printing alias data.
- Parallel read-only invocations of `notes folders` and `reminders list`
  completed without the previous redb-open failure.

### New release blocker found

- **Existing Reminders VFS body-only writes are ignored.** Repro:
  1. Create a reminder `cp orig <stamp>` in a disposable list.
  2. Write a local file containing only `cp renamed <stamp>` to
     `icloud:/Reminders/<list>/cp orig <stamp>.md` with `icloud cp`.
  3. `icloud reminders list all --list <list>` still shows the original title.

  This violates `crates/icloud-bash/SPEC.md`, which says the Reminders file
  body is the title and changing the body triggers a rename. The code path in
  `ICloudFs::write_file` only calls `edit_reminder()` for existing reminders
  when frontmatter fields changed (`due`, `notes`, `priority`, `completed`),
  so a title/body-only update is dropped. Frontmatter updates still work and
  also rename correctly because they force the edit path.

### Cleanup state

- Disposable active Notes folders, Reminders lists, and Reminders from this
  re-smoke were removed.
- Notes delete/folder cleanup still uses Apple Notes trash semantics, so any
  disposable Notes moved to Recently Deleted must age out or be removed
  manually from Apple Notes.

### Next tasks

1. Fix `crates/icloud-bash/src/vfs.rs` Reminders existing-file writes so a body
   change always calls `edit_reminder(id, Some(title), ...)`, even when no
   writable frontmatter fields changed.
2. Add unit/integration coverage for Reminders VFS writes:
   - existing reminder body-only write renames the reminder
   - frontmatter-only write updates notes/priority/due without changing title
   - redirect-created reminder receives command output instead of silently
     keeping the initial filename-derived title
3. Re-run the live disposable smoke:
   - `icloud cp body-only.md icloud:/Reminders/<list>/<old>.md`
   - `icloud bash -c 'echo new-title > /Reminders/<list>/<old>.md'`
   - verify `icloud reminders list all --list <list>` and VFS paths converge.

## 2026-06-13 - Installed CLI live smoke findings

Smoke-tested the installed binary at `/home/snowbear/.local/bin/icloud`
(`icloud 0.1.0`) against the already-authenticated account with disposable
`codex-*` Notes folders, Notes, Reminders lists, and Reminders.

### Verified

- Session validation: `icloud --quiet whoami` returned `ok`.
- Root VFS and `/tmp`: `icloud bash -c 'ls /'` showed
  `HideMyEmail, Notes, Reminders, tmp`; `/tmp` redirect/read worked.
- Notes CLI: folder create, note create, `notes get` Markdown body, update,
  move, and `notes list` visibility all worked.
- Notes VFS: reading an existing moved note worked; `mkdir /Notes/<folder>`
  created a folder visible to `notes folders`; single-quoted paths with spaces
  worked for redirect-created Notes files.
- `icloud cp`: host Markdown file -> `icloud:/Notes/...` -> host file preserved
  body content.
- Reminders CLI: list create, add, edit, list, and complete worked in the full
  smoke when operations were naturally spaced by other network calls.
- Reminders VFS: reading an existing reminder and redirect-creating a new
  reminder worked with single-quoted paths.
- Search: `icloud search <marker> --service notes --rebuild` found the created
  Markdown note.
- Local gates: `cargo test --workspace --locked`, `cargo fmt --all -- --check`,
  and `cargo clippy --workspace --all-targets -- -D warnings` all passed.

### Release blockers found

- **Bashbox path escaping:** backslash-escaped paths with spaces fail. Example:
  `icloud bash -c 'echo hello > /tmp/foo\ bar; cat /tmp/foo\ bar'` creates a
  literal `foo\ bar` entry and then cannot read `/tmp/foo bar`. Single-quoted
  paths work. This affects generated shell commands for Notes/Reminders titles
  or folders with spaces.
- **Notes folder delete lies:** `icloud notes folders <name> --delete --force`
  prints `Deleted folder ...` but only deletes/moves notes in the folder; it
  never calls `NotesSyncEngine::delete_folder`, so empty folders remain after
  sync. `icloud bash -c "rmdir '/Notes/<name>'"` does call the engine method and
  removed the smoke folders.
- **Recently Deleted leaks into `notes list`:** `notes delete` moves records to
  `TrashFolder-CloudKit`; subsequent `notes sync && notes list` still shows
  those notes under `Recently Deleted` because `get_notes()` only filters the
  `Deleted` field, not the trash folder.
- **CloudKit retry gap:** a focused add -> edit -> complete sequence hit
  `HTTP 409 ZONE_BUSY` / `CAS Op-Lock failed`. `with_reminders_retry` missed it
  because retry detection looks for `oplock` but not `op-lock`, `zone_busy`, or
  CAS wording.
- **Concurrent CLI DB open:** running Notes commands concurrently can fail with
  `redb open: Database already open. Cannot acquire lock.` The current lock
  file guards load/save sections, but redb itself rejects simultaneous opens.
- **List deletion with children:** `reminders lists <name> --delete --force`
  fails with CloudKit `VALIDATING_REFERENCE_ERROR` if the list still contains
  reminders. User-facing delete needs either a recursive delete path or a clear
  preflight error.

### Cleanup state

- Active smoke Notes folders, Reminders lists, and Reminders were removed.
- Four disposable smoke Notes remain in Apple Notes `Recently Deleted` because
  the CLI currently has no hard-purge operation. They use `codex-smoke-*`
  titles and should age out or be removed manually in Apple Notes.

### Next tasks

1. Patch bashbox in `../bashbox` to handle backslash escapes in words and
   redirection targets like Bash, add regression tests for `/tmp/foo\ bar`, push
   the fork, then bump the pinned rev in both Cargo.toml files.
2. Fix `cmd_notes.rs` folder deletion to delete contained notes, then call
   `NotesSyncEngine::delete_folder`; add a CLI/unit test around the command
   handler if possible and live-verify with `notes sync && notes folders`.
3. Treat `TrashFolder-CloudKit` as non-active in `NotesSyncEngine::get_notes`,
   VFS listings, search indexing, and exports unless an explicit
   `--include-deleted`/trash mode is added.
4. Expand CloudKit retry detection to include `zone_busy`, `CAS Op-Lock`,
   `op-lock`, and HTTP 409 retry hints; add a small backoff/jitter before sync
   + retry.
5. Add process-level DB open serialization or retry/wait behavior around redb
   open so parallel CLI invocations block briefly instead of failing.
6. Make reminder list deletion either recursively delete child reminders first
   or fail before the CloudKit call with a clear message and a documented
   `--recursive`/`--delete-reminders` option.

## 2026-06-13 - Release blockers fixed

Implemented the correction plan from the installed CLI smoke test.

- Patched `../bashbox` so normal word expansion removes the escape backslash
  for escaped characters while preserving escaped glob metacharacter semantics.
  Added `echo data > /tmp/foo\ bar; cat /tmp/foo\ bar` as a regression test.
  Pushed `OlegHQ/bashbox` `dev` at
  `2c4993bd7777cc648e8084729ee166e934dc38b1` and pinned both workspace
  `bashbox` dependencies plus `Cargo.lock` to that revision.
- Added a shared Notes active-record predicate that excludes
  `TrashFolder-CloudKit`, then used it for Notes CLI listings, search
  indexing, VFS listings/globs, and folder-delete emptiness checks.
- Fixed `icloud notes folders <name> --delete --force` to call
  `NotesSyncEngine::delete_folder` after moving/deleting contained active notes.
- Expanded CloudKit retry classification for `ZONE_BUSY`, `CAS Op-Lock`,
  hyphenated `op-lock`, and retry-hint wording, with short retry backoff before
  resync.
- Serialized redb opens behind the existing lock file so parallel CLI commands
  wait instead of failing with `Database already open`.
- Made `icloud reminders lists <name> --delete --force` recursively delete child
  reminders first, ordered deepest child before parent, then delete the list.
- Reinstalled the fixed CLI with
  `cargo install --path crates/icloud-cli --root /home/snowbear/.local --force --locked`.

### Verified

- `cargo test --workspace --locked`
- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `../bashbox`: `cargo test test_backslash_escaped_space_in_redirect_target --lib`
- `../bashbox`: `cargo test --lib` (2120 passed, 4 ignored)
- Live disposable smoke against `target/debug/icloud`:
  - `/tmp/foo\ bar` redirect/read works.
  - `/Notes/<folder with spaces>/<title with spaces>.md` redirect/read works
    with backslash-escaped paths.
  - Notes folder delete removes the folder after sync and the trashed note no
    longer appears in `notes list`.
  - Reminders list delete removes parent/child reminders before deleting the
    list.
  - Quick add -> edit -> complete reminder sequence succeeds.
  - Parallel `notes folders` and `notes list` complete without redb-open
    failure.
- Installed binary check:
  - `/home/snowbear/.local/bin/icloud version`
  - `icloud --max-age 0 --no-input bash -c 'echo ok > /tmp/install\ check; cat /tmp/install\ check'`

### Remaining risks

- Apple Notes still retains deleted Notes in Recently Deleted. Normal CLI/VFS
  surfaces now hide them, but there is still no hard-purge command.
- The lock-file approach serializes cache opens conservatively. This favors
  correctness for automation over concurrent read throughput.
- `Cargo.lock` currently contains yanked `fastrand 2.4.0`; locked install still
  succeeds, but dependency refresh should be considered before a public release.

## 2026-06-13 - Release readiness

- Removed the tracked `.envrc` Homebrew `LIBRARY_PATH` workaround; native Linux
  builds pass with `LIBRARY_PATH` unset, and `.envrc` is now ignored as a local
  developer file.
- Added root release metadata: `README.md`, `LICENSE`, `rust-toolchain.toml`,
  package repository metadata, CI workflow, tag-based release workflow, and
  hook installer scripts for POSIX shells and PowerShell.
- Added `.githooks/pre-push` to auto-run `cargo fmt --all` and
  `cargo clippy --fix` before `cargo test`; if fixes are applied, the hook
  stops the push so the changes can be reviewed and committed.
- Made the workspace clippy-clean under `cargo clippy --workspace --all-targets
  -- -D warnings` by fixing repeated resolver type complexity and mechanical
  `filter_map(...then...)`/signature lints.

### Verified

- `env -u LIBRARY_PATH cargo check --workspace`
- `cargo fmt --all`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test --workspace --locked`

### Remaining risks

- Local check-only builds for non-native x86_64 Linux, Windows GNU, and macOS
  targets failed because this ARM Linux host lacks the matching C
  cross-compilers/Apple SDKs needed by `ring`; the release workflow uses native
  GitHub runners instead.
- GitHub Actions macOS labels should be monitored over time because hosted
  runner labels change; the current workflow uses `macos-26` for Apple Silicon
  and `macos-26-intel` for Intel.

## 2026-04-27 — IMPROVEMENT.md sweep

Implemented Phase 0/1/2/3 items from `IMPROVEMENT.md` (P0-4 deferred — needs an
upstream `bashbox` patch). Items addressed:

- **P0-1** `find_item` canonical IDs round-trip (`crates/icloud-api/src/store.rs`).
  Strip the prefix from both sides and switch the suffix-match from `contains`
  to `starts_with`. 5 unit tests under `store::find_item_tests`.
- **P0-2** `/tmp` is pre-seeded inside `ICloudFs::new` so `[ -d /tmp ]`,
  `mkdir /tmp/sub`, and shell redirects under `/tmp` work without relying on
  bashbox's default-cwd layout. `ICloudFs::new` is now `async`; both
  `cmd_bash.rs` and `cmd_cp.rs` already `await`.
- **P0-3** Every CLI mutation in `cmd_reminders.rs` and `cmd_notes.rs` now
  wraps the engine call in `with_reminders_retry` / `with_notes_retry`. Stale
  `recordChangeTag`/`OP_LOCK_FAILURE` no longer surface to the user.
- **P1-1** `add_reminder` and `add_reminders_batch` return the canonical
  `Reminder/<UUID>` (singular) and `Vec<String>` (batch); CLI emits these as
  `id` / `ids` in `--json`, plus `id` for `notes create`.
- **P1-2** `print_ok`, `print_ok_with`, `print_hme_action` now emit human
  success on **stdout**; `hint(...)` continues to use stderr. `output.rs`
  documents the stream contract at the top.
- **P1-3** `print_whoami` and `handle_hme` accept `OutputMode`; `--plain`
  emits TSV (whoami: 4 fields; HME list: 7 fields) and `--quiet` emits a
  count for HME list / `ok|failed` for whoami.
- **P2-1** `/Attachments` is hidden from root readdir; stat/read/readdir all
  return `NotFound` consistently. SPEC.md notes this as future work.
- **P3-1** `Edit::due` help text matches `Add::due`; both now advertise
  `today, tomorrow, yesterday, YYYY-MM-DD` (the only forms `str_to_ts`
  actually accepts).
- **P3-2** `notes folders <name> --create` parity with `reminders lists`.
  `--rename` was deliberately not added because the API lacks
  `rename_folder`.
- **P3-3** Top-level `--help` ends with an `EXIT CODES:` section that matches
  the source mapping (0 success, 2 usage, 3 auth, 4 upstream).
- **P3-4** `icloud version` prints text in human mode and a small JSON
  object (`{name, package, version}`) with `--json`.
- **P3-5** `--body` long help documents the verbatim semantics and points
  scripts at stdin for multi-line payloads.
- **P3-6** `prompt_2fa` now refuses interactively when `--no-input` is set
  (or `--json`), surfacing a clean auth error instead of blocking on stdin.

Not done in this pass:

- **P2-2** Live verify that `mkdir /Notes/<folder>` shows up in
  `notes folders` after a sync — verified live with `e2e-folder-...`,
  see live-e2e section below.
- New unit-test scaffolds for `output.rs` stream destinations (P1-2 tests
  in IMPROVEMENT.md) — not added because they require capturing stdout
  without disturbing the existing mutation tests; existing format-shape
  contracts are covered indirectly by the new `id` JSON fields.

### Verified

- `cargo fmt --all`
- `cargo build --workspace` (warning-clean)
- `cargo test --workspace` — 67 passing, 1 ignored
- `cargo run -p icloud-cli -- version` and `--json version`
- `cargo run -p icloud-cli -- --help` (exit-code section appears)

### Live e2e against `oleg@nexo.sh`

- `whoami` (default, `--plain`, `--quiet`) — all three modes formatted
  correctly (P1-3).
- Reminder full round-trip on the canonical `Reminder/<UUID>` returned
  by `add -j`: `add → edit --priority → complete → delete --force`
  (P0-1, P1-1, P0-3).
- `reminders delete <UUID-prefix> --force` resolves via `find_item`
  suffix path (P0-1).
- `reminders add-batch ... -j` returns `ids: [...]` (P1-1); both
  reminders deleted by canonical id.
- Stream contract: `reminders add` writes "Added: ..." to stdout and
  the hint to stderr (P1-2).
- HME `--quiet` prints `88` (count); `--plain` emits 7-field TSV (P1-3).
- `icloud bash -c '[ -d /tmp ] && echo TMP_DIR_OK; mkdir /tmp/sub'`
  works; `/tmp` is no longer ENOENT (P0-2).
- `ls /` no longer lists `Attachments`; `ls /Attachments` returns
  ENOENT (P2-1).
- `notes create` (stdin markdown) returns `id`+`folder`+`title`; `notes
  get <uuid>` retrieves the body; `notes delete <uuid>` succeeds
  (P0-1, P1-1).
- `notes folders <name> --create -j` returns the new folder id;
  `notes sync && notes folders --json` lists it; `notes folders <name>
  --delete --force` cleans it up (P3-2, P2-2 verified live).
- `reminders complete Reminder/00000000-...` returns exit code `2`
  (`EXIT_USAGE`), aligned with the help-text mapping (P3-3).
- 3-iteration `add → edit → complete → delete` loop produced **no**
  `oplock`/`conflict`/`stale`/`changeTag` errors on stdout/stderr
  (P0-3).
- `bash -c 'echo data > /tmp/x && cat /tmp/x'` → `data`
  (P0-4: bashbox redirect wiring).
- `bash -c 'echo one > /tmp/x; echo two >> /tmp/x; cat /tmp/x'` →
  `one\ntwo` (P0-4 append).
- `bash -c 'ls /nope 2> /tmp/e; cat /tmp/e'` → `ls: cannot access ...`
  via the file (P0-4 stderr redirect).
- `bash -c 'echo redirect-data > /Notes/<folder>/test.md && cat ...'`
  created the note in iCloud and the body roundtripped (P0-4 +
  iCloud VFS).

### Bashbox (`OlegHQ/bashbox`)

- Patched `interpreter/execution_engine.rs` to call
  `pre_open_output_redirects` before dispatch and `apply_redirections`
  after, for both `Simple` and `Compound` commands. Added
  `collect_simple_command_redirects` for the suffix/prefix flatten.
- Added 7 redirect tests (`bash::tests::test_redirect_*`); library suite
  is now 2119 passing (was 2112).
- Pushed as `ee1080a509aeb5e09e084c998a318a6976247f13` and pinned in
  both `crates/icloud-bash/Cargo.toml` and
  `crates/icloud-cli/Cargo.toml`.

Pre-existing clippy lints remain in `icloud-api` (`type_complexity`,
`filter_map_bool_then`, `useless_vec`) and `icloud-bash`/`icloud-cli`
(`filter_map_bool_then`, `redundant_else`, …). They were present on the
pre-change baseline and were not introduced by this work; one redundant
closure I introduced in `output.rs` was removed.

### Remaining risks

- The closure pattern around `with_*_retry` clones strings on every retry.
  That's fine for the current call rate (one mutation per CLI invocation)
  and intentional — the helper signature requires `'static` futures.
- `notes create -j` derives `title` from the first markdown line by
  stripping leading `#` chars; it does not normalise to whatever Apple
  Notes ends up storing as the title. The `id` field is authoritative.
- HME `list` plain/quiet output is now stable; consumers that previously
  scraped the pretty-printed JSON (default human mode) need to switch to
  `--json`.

### Next tasks

- Land the bashbox redirect wiring (P0-4) so `>`, `>>`, and `2>` actually
  hit the configured `FileSystem`. Without it, `icloud bash -c 'echo data
  > /tmp/x'` still drops the data on the floor.
- Live-verify Notes folder creation (P2-2) and add a regression for the
  cache-freshness path if needed.
- Consider an `output_contract` unit-test crate that captures stdout/
  stderr via redirect to lock the P1-2 contract in.

## Current architecture

- Workspace crates: `icloud-api`, `icloud-cli`, `icloud-bash`.
- `icloud bash` embeds **[bashbox](https://github.com/OlegHQ/bashbox)** (pinned git dependency in `Cargo.toml` files). A vendored `crates/just-bash/` tree is **not** part of this repository anymore.
- `icloud-bash::ICloudFs` implements `bashbox::fs::FileSystem`, composing Notes/Reminders sync engines, Hide My Email, and `bashbox::InMemoryFs` for `/tmp` and non-iCloud paths.

## What to update here

After substantial VFS, `icloud bash`, or bashbox integration work, record:

- what changed
- what was verified (commands run)
- what remains risky or incomplete
- sensible next tasks

## References

- Agent policy: `AGENTS.md`
- VFS semantics and roadmap detail: `crates/icloud-bash/SPEC.md`

## 2026-04-09

- Investigated `/Users/snowbear/Downloads/search.har` plus the current iCloud Notes web bundle to look for a dedicated remote Notes text-search endpoint. The web app still uses CloudKit `records/query` for `recents`, `parentless`, `pinned`, and reference lookups, then builds a local `Indexer` for note search once a search term is entered.
- Reworked the cold Notes search path so `icloud search` no longer fetches full note bodies one note at a time just to build the local FTS index. Notes search now batches `TextDataEncrypted` lookups, decodes plain searchable text directly from the note protobuf, and persists that `search_text` alongside cached Markdown bodies.
- Search still prefers cached `body_markdown` when available, so note reads and edited notes keep richer search coverage, but untouched notes can now be indexed without the attachment/table expansion path.
- Notes write paths now keep `search_text` coherent for created and updated notes, and Notes cache merges preserve `search_text`/`body_markdown` when metadata is unchanged across syncs.

### Verified

- `cargo fmt`
- `cargo check`
- `cargo test -q -p icloud-api decodes_note_body_into_search_text`
- `cargo test -q search_roundtrip_filters_by_scope`
- `cargo test -q -p icloud-cli infers_reminders_from_scope`
- `cargo test -q -p icloud-api roundtrip_empty_and_nonempty`

### Remaining risks

- There is still no evidence of a dedicated remote Notes full-text endpoint in the current web client. Search remains a local-index design, just with a much cheaper cold-start hydration path.
- Batched search hydration decodes the note body text but does not resolve table contents or attachment titles unless the full Markdown body was already fetched elsewhere. Search quality for those attachment-heavy notes may therefore be slightly worse than the previous full-body path.
- No live-account measurement was done after the batching change, so the actual speedup versus the old one-record-per-request path still needs manual validation.

### Next tasks

- Manually benchmark `icloud search <term> --service notes` on a cold Notes cache after deleting any persisted search index to confirm the batching win on a real corpus.
- If attachment-heavy notes search materially worse, add an optional second-stage enrichment pass for cached attachment/table text instead of restoring per-note cold fetches.
- Decide whether `search` should default to a longer cache age than the global `--max-age 5` now that local Notes indexing is cheaper but still gated by sync freshness.

## 2026-04-08

- Added a pure-Rust full-text search layer in `crates/icloud-api/src/search.rs` using Tantivy on top of the existing `redb` caches instead of replacing the cache store.
- Notes now persist cached Markdown bodies in `NotesCache`, reuse them on reads/search, and invalidate them when sync metadata changes. This makes full-text search cover real note file contents without refetching every time.
- Added top-level `icloud search` with service/path scoping over VFS paths such as `/Notes/Work` and `icloud:/Reminders/Home`.
- Split the search index into per-service Tantivy directories and taught `icloud search` to infer the needed services from `--service` and `--path`, so `/Reminders/...` searches no longer open Notes or hydrate note bodies first.
- `icloud search` now opens/syncs Notes and Reminders concurrently when both are needed, and skips index refresh entirely when the service cache timestamp matches the last indexed state. Repeated warm-cache searches now hit the local index directly instead of re-materializing all indexed documents first.
- Added top-level `icloud cp` for host<->iCloud transfers with explicit `icloud:/...` endpoints, recursive directory copy for supported VFS shapes, and UTF-8/Markdown validation for Notes/Reminders imports.
- Hardened `icloud-bash::ICloudFs` so `stat()/exists()` consult real cache state instead of accepting any syntactically valid VFS path, and so note/reminder writes reject non-UTF-8 payloads instead of lossy-converting them.

### Verified

- `cargo check`
- `cargo test`
- `cargo run -q -p icloud-cli -- search --help`
- `cargo run -q -p icloud-cli -- cp --help`
- `cargo test -q search_roundtrip_filters_by_scope`
- `cargo test -q -p icloud-cli infers_reminders_from_scope`
- `cargo test -q -p icloud-cli rejects_non_searchable_scope`
- `cargo test -q -p icloud-cli rejects_conflicting_service_scope`
- `cargo check` after parallel search open + cache-marker refresh changes

### Remaining risks

- `icloud search` hydrates any uncached note bodies on first use. That is correct for coverage, but it can be slow on large accounts because body fetches are still one note at a time.
- Cross-service search now merges top hits from separate Notes/Reminders indices. That removes unnecessary service work, but combined relevance ordering is only approximately comparable across the two Tantivy indices.
- Notes full-text search is still expensive on the first corpus-wide build because uncached note bodies are fetched one note at a time before indexing.
- `icloud cp -r` intentionally mirrors the current VFS shape, not the deeper native Notes folder tree. Import into `/Notes` or `/Reminders` expects immediate child directories that map to folders/lists; nested host directories under a single folder/list are rejected.
- No live-account manual validation was done in this session, so real CloudKit mutation behavior for `icloud cp` still needs smoke testing against an account.

### Next tasks

- Manually test `icloud search` on a real account with cold and warm caches, and measure first-run hydration time on a larger Notes corpus.
- Manually verify that reminders-only searches (`icloud search ... --path /Reminders/...`) skip Notes sync/body hydration on a live account and return promptly with stale and fresh caches.
- Manually measure warm-cache search latency with the new cache-marker fast path and decide whether `search` should get a less aggressive default than the global `--max-age 5`.
- Manually test `icloud cp` in both directions for single files and `-r` directory copies, especially folder/list auto-creation and reminder frontmatter validation failures.
- Consider batching or parallelizing note body hydration if first-run search latency is too high.

## 2026-04-09

- Search results now expose reminder due dates in the human table, and reminder hits carry `due` through the local Tantivy index.
- Search ordering now prefers upcoming reminder files ahead of overdue ones, while leaving note and list hits in the relevance-sorted middle tier.
- Added a targeted search test to verify that upcoming reminders sort ahead of past-due reminders for identical query text.
- Search output is now rendered as stacked result blocks instead of a wide table, so long paths and snippets do not compress the layout on narrow terminals.
- Reminder due dates now only render on reminder hits; note hits stay focused on title, type, path, and snippet.

### Verified

- `cargo fmt --all`
- `cargo check`
- `cargo test -q -p icloud-api search_roundtrip_filters_by_scope`
- `cargo test -q -p icloud-api search_prioritizes_upcoming_reminders`

### Remaining risks

- Reminder sort priority is still an approximation based on due date buckets plus relevance; it does not account for completion state yet.
- Search output is still a plain table, so very long reminder paths can crowd out the due/snippet columns on narrow terminals.
- The block-style search output is better for readability, but it still has no explicit wrapping or truncation controls.
- Reminder hits are still ranked by due-date bucket plus relevance; if that remains noisy, completion state is the next obvious tie-breaker.

### Next tasks

- Consider giving completed reminders a lower rank than overdue-but-open reminders if the current ordering still feels noisy in practice.
- Consider a more compact search row format or optional column toggles if the table is still too wide for real-world reminder-heavy queries.

## 2026-04-09

- `icloud bash` now intercepts `exit` in the interactive REPL and exits the shell instead of handing the builtin to bashbox and then ignoring the signal.
- Added a host-side `edit <path>` helper in the REPL: it writes the current VFS file to a temp file, opens `$VISUAL`/`$EDITOR` (fallback `vi`), and writes the edited contents back only when the editor exits successfully.

### Verified

- `cargo fmt --all`
- `cargo check`

### Remaining risks

- The editor helper assumes the chosen editor blocks until the user closes the file. GUI editors such as VS Code need `--wait` in `EDITOR`/`VISUAL` to behave correctly.
- `edit` is only wired in the interactive REPL today; it is not yet a bashbox builtin or a non-interactive script command.

### Next tasks

- Consider adding a dedicated `icloud edit` command if the same host-editor workflow should be available outside the bash REPL.
- Consider adding a small regression test around REPL builtin parsing once the interactive path is factored into a testable helper.
