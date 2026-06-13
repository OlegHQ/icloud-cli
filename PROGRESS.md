# Progress

Handoff log for cross-session work. New sessions should skim this before planning larger changes.

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
