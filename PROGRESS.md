# Progress

Handoff log for cross-session work. New sessions should skim this before planning larger changes.

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
