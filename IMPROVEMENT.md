# IMPROVEMENT.md — Locked-down finalization plan for `icloud`

Status: re-verified 2026-04-27 against the source tree at `dev`. Every claim
below is anchored to a `path:line` reference and was checked, not paraphrased.
Changes from the prior draft are summarised in the **What changed in this
revision** section and folded into the per-item entries.

This is the implementation contract. Open PRs against the items in the
**Implementation order** section. Do not start the iCloud-CLI Claude skill
(`./skills/icloud-cli`) until all Phase 0 + Phase 1 items here ship — the skill
documents the surface, and the surface is wrong today.

---

## TL;DR

- 4 P0 (contract-breaking) bugs, all reproducible against a live account.
- 1 of them lives in upstream `OlegHQ/bashbox` (redirect support is **dead
  code**, never wired into the execution engine).
- 3 P1 bugs make machine output unusable for agents/scripts.
- A handful of P2/P3 ergonomics and several feature gaps that should be filed
  as their own issues, not gates to "v1".

Release gate at the bottom of this document.

---

## Goal (same as before, restated)

Make the CLI boringly reliable:

1. IDs emitted in `--json` round-trip to every command that takes IDs.
2. Mutation commands return useful machine-readable results, including new
   record IDs.
3. **stdout** carries command results; **stderr** carries diagnostics,
   progress, prompts, and hints. Always.
4. `icloud bash` exposes a VFS whose listed paths actually work. `>` and `>>`
   redirect into iCloud and `/tmp`.
5. CloudKit change-tag conflicts are retried transparently.
6. `--plain` works for every read command.
7. Help text and tests describe the **real** contract.

---

## What changed in this revision

| Item | Earlier draft | Verified now |
|---|---|---|
| P0-1 | Said "find_item compares verbatim against suffix, canonical IDs miss". | **Confirmed.** `crates/icloud-api/src/store.rs:139` strips the prefix from the **cache key** but not from `partial`, so `Reminder/<UUID>.contains("reminder/<uuid>")` fails. Same path is used by Notes via `find_note → find_item`. |
| P0-2 | Said `/tmp` is delegated to InMemoryFs and not created. | **Confirmed and root-caused.** Bashbox `bash.rs:46` sets `use_default_layout = options.cwd.is_none()`. We pass `cwd: Some("/")`, so `init_filesystem` (`bash.rs:225`) **skips** the `/tmp` mkdir. The same gap exists in `crates/icloud-cli/src/cmd_cp.rs:101` where ICloudFs is constructed identically. |
| P0-3 | Said "wrap CLI mutations in `with_*_retry`". | **Confirmed and broader.** `crates/icloud-cli/src/cmd_reminders.rs` and `cmd_notes.rs` both call `engine.<op>().await` directly (lines 217, 305, 326, 341, 357 in cmd_notes; 300, 316, 321, 367, 401, 447, 510, 543, 549, 555 in cmd_reminders). VFS mutations already use `with_*_retry` (`vfs.rs:512, 522, 564, 571, 590, 617, 777, 788, 902, 922, 933, 944, 991, 1057, 1066, 1076, 1110`). |
| P0-4 | Said "redirects don't call FileSystem::write_file in bashbox". | **Wrong on the mechanism, right on the symptom.** Bashbox already has `pub fn apply_redirections` and `pub fn pre_open_output_redirects` in `src/interpreter/redirections.rs` that *do* call `FileSystem::write_file` / `append_file` correctly. **Both functions are dead code** — never called from `execute_simple_command` (`execution_engine.rs:368-417`), `execute_compound_command`, or anywhere else in bashbox. The fix is wiring, not patching. |
| P1-1 | Said "API knows the IDs but CLI discards them". | **Confirmed.** `add_reminder` returns `Result<()>` (`reminders/write.rs:14-78`); the canonical `record_name = "Reminder/<UUID>"` already exists locally. `create_note` already returns `Result<String>` (`notes/write.rs:141`); CLI just ignores the value (`cmd_notes.rs:305`). |
| P1-2 | Said "human-mode mutation results print to stderr". | **Confirmed.** `output.rs:319` (`print_hme_action`), `output.rs:331` (`print_ok`), `output.rs:343` (`print_ok_with`) all use `eprintln!` for human success. Hints (`output.rs:88`) are correctly on stderr. |
| P1-3 | Said "whoami and HME ignore --plain". | **Confirmed.** `output.rs:279 fn print_whoami(json: bool, ...)` and `cmd_hme.rs:53 fn handle_hme(json: bool, ...)` both take a bool, not `OutputMode`. |
| Exit codes | Earlier draft proposed `1=usage, 2=domain, 3=auth, 4=transport, 5=cache`. | **Source disagrees.** `crates/icloud-cli/src/main.rs:29-48` defines `EXIT_USAGE=2, EXIT_AUTH=3, EXIT_UPSTREAM=4`. Updated P3-3 to align help text with **source**, not the other way around. |
| Notes folders | Earlier draft asked "live-verify mkdir". | **Source has `create_folder` returning `Result<String>`** (`notes/write.rs:451`). The VFS calls it via `with_notes_retry` (`vfs.rs:777`). Need a live smoke test, not an API change. |
| `/Attachments` | Earlier draft: hide it. | **Decision held.** `vfs.rs:827` lists it from root. `cmd_search.rs:267` already rejects it as a search scope. Hide from root listing; keep `VfsTarget::Attachments*` so error mapping stays clean. |

New items added in this revision (from the audit): exit-code alignment, Notes
folder CLI parity with Reminders Lists, HME `reactivate`, `--no-input` checked
in 2FA prompt, `--quiet` propagation. See Phase 3 / Phase 4.

---

## Design principles

- **Source of truth is the source.** Every claim in this document carries a
  `path:line` reference. If the line is wrong, fix the doc; don't fix the code
  to match a stale doc.
- **Don't break wire formats; do break CLI shape between v0 and v1.** No JSON
  Schema, no migration of cache files. Users re-login and re-sync (per
  `AGENTS.md` line 16). The CLI is pre-1.0; we change the surface where
  needed.
- **No host filesystem from the embedded shell.** Redirect targets must always
  resolve through the configured `FileSystem`, never `std::fs`.
- **Two retry attempts is enough.** `is_cloudkit_retryable` already covers the
  CloudKit conflict/oplock vocabulary (`retry.rs:8-16`). Don't broaden it.
- **stdout=results, stderr=everything else.** Hints, progress, prompts,
  warnings, dry-run previews → stderr. Mutation success messages and JSON
  payloads → stdout.

---

## Phase 0 — Contract-breaking bugs

### P0-1. Canonical IDs must round-trip through `find_item`

**Where.** `crates/icloud-api/src/store.rs:139-155`.

**Bug.** `find_item` strips the cache key's prefix (`Reminder/<UUID>` →
`<UUID>`) before its `contains` check, but does **not** strip the input.
Result: `find_item("Reminder/ABC")` searches for `"reminder/abc"` inside the
already-stripped `"abc"` — and never matches.

```rust
// store.rs:139
fn find_item(&self, partial: &str) -> Option<String> {
    let p = partial.to_ascii_lowercase();
    for (id, item) in self.items() {
        if Self::item_title(item).to_ascii_lowercase() == p {
            return Some(id.clone());
        }
    }
    for id in self.items().keys() {
        let check = id.rsplit_once('/').map(|(_, u)| u).unwrap_or(id);
        if check.to_ascii_lowercase().contains(&p) {  // ← BUG
            return Some(id.clone());
        }
    }
    None
}
```

This affects **all** Reminder mutation paths (`complete_reminder`,
`uncomplete_reminder`, `delete_reminder`, `edit_reminder`, all of
`reminders/write.rs`) and Note mutation paths via the shared `find_item`
through `find_note`.

**Fix.**

```rust
fn find_item(&self, partial: &str) -> Option<String> {
    let p = partial.to_ascii_lowercase();
    // Exact title match wins before any ID heuristic.
    for (id, item) in self.items() {
        if Self::item_title(item).to_ascii_lowercase() == p {
            return Some(id.clone());
        }
    }
    // Strip the same prefix from both sides so `Reminder/<UUID>` matches.
    let p_suffix = p.rsplit_once('/').map(|(_, u)| u).unwrap_or(&p);
    for id in self.items().keys() {
        let key_suffix = id.rsplit_once('/').map(|(_, u)| u).unwrap_or(id);
        if key_suffix.eq_ignore_ascii_case(p_suffix)
            || key_suffix.to_ascii_lowercase().starts_with(p_suffix)
        {
            return Some(id.clone());
        }
    }
    None
}
```

Notes:
- We change `contains` to `starts_with` for the suffix path. `contains` accepts
  `"BC"` as a match for `"ABC..."`, which is too loose for an "ID prefix"
  contract. Real users either pass the full canonical ID or an opening prefix
  of the UUID. If anyone relied on substring matching of UUIDs, that's a
  feature loss we accept.
- Keep title precedence above ID precedence so that
  `find_item("Buy milk")` wins when a reminder has that exact title.

**Tests** (in `crates/icloud-api/src/store.rs` `#[cfg(test)] mod tests`,
parameterised over both Reminders and Notes caches via a tiny helper):

- `find_item_accepts_canonical_record_id`
- `find_item_accepts_bare_uuid_prefix`
- `find_item_preserves_exact_title_precedence`
- `find_item_is_case_insensitive_for_ids`
- `find_item_rejects_unrelated_substring`

**Acceptance.**

```bash
id=$(icloud reminders list -l Reminders --json | jq -r '.[0].id')
test -n "$id"
icloud reminders edit "$id" --priority high
icloud reminders complete "$id"
icloud reminders delete "$id" --force
```

The earlier draft's `--force` requirement on a single delete is correct —
`cmd_reminders.rs:497` only prompts when `ids.len() > 1`, but `--force` also
suppresses the prompt and is the right idiom for scripts.

---

### P0-2. Pre-seed `/tmp` so `[ -d /tmp ]` and shell writes work

**Where.**
- `crates/icloud-cli/src/cmd_bash.rs:48-51` (constructs `InMemoryFs` and `Bash`).
- `crates/icloud-cli/src/cmd_cp.rs:101-106` (same construction, easy to miss).
- `crates/icloud-bash/src/vfs.rs:213-227` (`ICloudFs::new`).

**Root cause.** Bashbox `Bash::new` (`bash.rs:46`):
```rust
let use_default_layout = options.cwd.is_none();
// ...
init_filesystem(&*fs, use_default_layout).await;  // bash.rs:115
```

`init_filesystem` only mkdirs `/tmp` and `/home/user` when
`use_default_layout` is true (`bash.rs:225-230`). We pass `cwd: Some("/")` to
get `HOME=/` and `pwd=/`, so the default layout helper is skipped. Result:
`/tmp` is listed by `ICloudFs::readdir` for `/` (`vfs.rs:826-832`) but the
underlying `InMemoryFs` has no `/tmp` entry, so `[ -d /tmp ]`, `mkdir /tmp/sub`,
and any redirect target under `/tmp` fail with `ENOENT`.

**Fix.** Pre-seed `/tmp` in `ICloudFs::new` (so all callers get it for free,
including `cmd_cp.rs`) and seed it lazily in `read_file_buffer`/`stat`/`mkdir`
fallthroughs only if needed.

Concrete change:

```rust
// crates/icloud-bash/src/vfs.rs
impl ICloudFs {
    pub async fn new(
        inner: Arc<InMemoryFs>,
        notes: Arc<Mutex<NotesSyncEngine>>,
        reminders: Arc<Mutex<SyncEngine>>,
        hme: Arc<HideMyEmailClient>,
    ) -> Self {
        // /tmp must exist because root readdir advertises it.
        let _ = inner
            .mkdir("/tmp", &MkdirOptions { recursive: true })
            .await;
        Self {
            inner,
            notes,
            reminders,
            hme,
            hme_cache: Arc::new(Mutex::new(HmeCache::default())),
        }
    }
}
```

`ICloudFs::new` becomes async. Update `cmd_bash.rs:51` and `cmd_cp.rs:101` to
`Arc::new(ICloudFs::new(...).await)`. Both already run inside `tokio::main`,
so this is mechanical.

**Tests** (`crates/icloud-bash/tests/golden_vfs.rs`):

- `tmp_is_a_directory_after_construction`
- `tmp_supports_subdir_create`
- `tmp_does_not_appear_under_icloud_prefixes` (regression: P2-3)

**Acceptance.**

```bash
icloud bash -c '[ -d /tmp ] && echo yes' | grep -qx yes
icloud bash -c 'mkdir /tmp/sub && tee /tmp/sub/a >/dev/null <<<ok && cat /tmp/sub/a' \
  | grep -qx ok
```

Note: this acceptance check works **before** P0-4 because `tee` calls the
async `FileSystem` directly. The `>` form below requires P0-4.

---

### P0-3. Wrap CLI mutations in `with_*_retry`

**Where.**
- `crates/icloud-cli/src/cmd_reminders.rs` — every mutation site
  (lines 300, 316, 321, 367, 401, 447, 510, 543, 549, 555).
- `crates/icloud-cli/src/cmd_notes.rs` — every mutation site
  (lines 217, 305, 326, 341, 357).
- `crates/icloud-api/src/retry.rs` — already exports the closure-based
  helpers we need; signature is `for<'a> FnMut(&'a mut SyncEngine) ->
  Pin<Box<dyn Future<Output = Result<T>> + Send + 'a>>`.

**Bug.** Every mutation in `cmd_*.rs` calls `r.engine.<op>().await?` directly.
On stale `recordChangeTag`, this surfaces `OP_LOCK_FAILURE` to the user.
`crates/icloud-bash/src/vfs.rs` already wraps every mutation, so the bash VFS
path is fine — the CLI is the inconsistent half.

**Fix.** For every mutation, replace the bare engine call with a closure:

```rust
// before:
r.engine.delete_reminder(id).await?;

// after:
icloud_api::with_reminders_retry(&mut r.engine, |e| {
    let id = id.clone();
    Box::pin(async move { e.delete_reminder(&id).await })
})
.await?;
```

There are 14 callsites total. Add a small helper in `cmd_reminders.rs` /
`cmd_notes.rs` if the repetition gets noisy, but **do not** push the wrapping
into the engine itself — the closure-pattern is intentional (the helpers need
to call `engine.sync(false)` between attempts and that requires `&mut self`).

`is_cloudkit_retryable` (`retry.rs:8-16`) already matches `oplock`, `conflict`,
`stale`, `record changed`, `server record changed`, and `changetag`. Don't
broaden it.

**Tests.**

- Unit (`retry.rs`): `is_cloudkit_retryable` recognises `OP_LOCK_FAILURE`,
  `Server record changed`, `stale changeTag`, `Conflict on record`. Add table-
  driven test with redacted error strings copied from real responses.
- Live ignored (`crates/icloud-cli/tests/live_smoke.rs`, gated on
  `ICLOUD_CLI_LIVE=1`): a 20-iteration add → edit → complete → delete loop
  that must not surface `OP_LOCK_FAILURE` to stdout/stderr.

**Acceptance.**

```bash
for i in $(seq 1 20); do
  id=$(icloud reminders add -l Reminders "retry-smoke-$i" -j | jq -r .id)
  icloud reminders edit "$id" --priority high
  icloud reminders complete "$id"
  icloud reminders delete "$id" --force
done 2>&1 | grep -qi 'oplock\|stale\|conflict' && exit 1 || echo ok
```

---

### P0-4. Wire `apply_redirections` and `pre_open_output_redirects` into the bashbox execution engine

**Where.**
- Upstream: `OlegHQ/bashbox` rev `ac7c34eebcc2efcf1895b566c72d9c3265ecad79`
  (pinned in `crates/icloud-cli/Cargo.toml` and
  `crates/icloud-bash/Cargo.toml`).
- The dead functions: `src/interpreter/redirections.rs` lines 194 and 333.
- The call sites that should invoke them: `src/interpreter/execution_engine.rs`
  `execute_simple_command` (line 368), `execute_compound_command` (line 420),
  and pipeline handling in `src/interpreter/pipeline_execution.rs`.

**Bug.** I re-checked this myself:

```bash
$ rg 'apply_redirections|pre_open_output_redirects' bashbox/
src/interpreter/redirections.rs:194:pub fn pre_open_output_redirects(
src/interpreter/redirections.rs:333:pub fn apply_redirections(
```

Two definitions. Zero call sites. The functions exist with the right shape —
they take `&dyn FileSystem`, expand redirect targets, write to
`fs.write_file` / `fs.append_file`, and clear `stdout`/`stderr` after. They
are simply never invoked from anywhere that runs commands.

I reproduced the user-visible symptom against the live CLI:

```bash
$ icloud bash -c 'echo data > /tmp/x && cat /tmp/x'
data
cat: /tmp/x: No such file or directory
```

`echo` printed `data` (because the redirect was never applied to clear
stdout) **and** `/tmp/x` was not written (because the redirect was never
applied at all). `tee` works only because `tee`'s builtin
(`bashbox/src/commands/tee/mod.rs:43-45`) calls
`ctx.fs.write_file(...).await` directly, bypassing the redirect machinery
entirely.

**Fix.** In `bashbox`:

1. In `execute_simple_command` (`execution_engine.rs:368-417`), extract the
   `IoRedirect` items from `cmd.suffix` (helper exists at
   `alias_expansion.rs:141-156`). Then:
   - Call `pre_open_output_redirects(...)` before dispatch — this truncates
     output files for `>`/`>|` so the file exists with empty contents even
     if the command produces no output.
   - After the builtin/external returns its `ExecResult`, call
     `apply_redirections(...)` to commit `result.stdout`/`stderr` to the
     filesystem and clear them out of the returned struct.
2. Do the same in `execute_compound_command` for `RedirectList` on compound
   commands (the AST already carries them at `bast::Command::Compound(_,
   redirects)`, line 334).
3. Pipeline redirection: per-stage redirects are already part of each stage's
   `SimpleCommand`, so step (1) covers them. Pipeline-level redirects (the
   trailing `>` on the whole pipeline) need wiring in
   `pipeline_execution.rs`.
4. Tests in bashbox using `InMemoryFs`: `>`, `>>`, `2>`, `&>`, `>&2`, `2>&1`,
   pipe with trailing redirect, redirect on compound command, redirect on
   function call.

**Workflow** (per `AGENTS.md` lines 122 and 226-238):

1. `git clone https://github.com/OlegHQ/bashbox ../bashbox` (no local
   checkout exists at `~/projects/bashbox` today; the cargo cache copy is
   read-only and not usable for editing).
2. Branch, fix, test (`cargo test --lib` inside bashbox).
3. Push and capture the new rev.
4. Bump `rev = "..."` in **both** `crates/icloud-bash/Cargo.toml` and
   `crates/icloud-cli/Cargo.toml` in lockstep.
5. `cargo build` at the workspace root so `Cargo.lock` updates.
6. Run the icloud-cli redirect acceptance tests.

**Acceptance.**

```bash
icloud bash -c 'echo data > /tmp/x && cat /tmp/x' | grep -qx data
icloud bash -c 'echo one > /tmp/x; echo two >> /tmp/x; cat /tmp/x' \
  | grep -Pzq 'one\ntwo\n'
icloud bash -c 'ls /nope 2> /tmp/e; cat /tmp/e' | grep -q 'No such'
icloud bash -c 'echo data > /Notes/Drafts/redirect-smoke.md' \
  && icloud notes get redirect-smoke | grep -q data
```

The last test exercises the iCloud write path through redirects, which is the
whole point of having a VFS-backed shell.

---

## Phase 1 — Machine-readable results and stream contract

### P1-1. Return created IDs from create operations

**Where.**
- `crates/icloud-api/src/reminders/write.rs:14` (`add_reminder`) — currently
  `Result<()>`.
- `crates/icloud-api/src/reminders/write.rs:80` (`add_reminders_batch`) —
  currently `Result<()>`.
- `crates/icloud-api/src/notes/write.rs:141` (`create_note`) — already
  `Result<String>`. CLI throws the value away.
- `crates/icloud-api/src/notes/write.rs:451` (`create_folder`) — already
  `Result<String>`. CLI doesn't expose folder creation yet (P3-2).
- `crates/icloud-cli/src/cmd_reminders.rs:367` (`Add`), `cmd_reminders.rs:401`
  (`AddBatch`), `cmd_notes.rs:305` (`Create`).

**Fix.**

API:
- Change `add_reminder` to return the canonical `Reminder/<UUID>`. The
  function already builds it locally as `record_name` (write.rs:52-55) — just
  return it.
- Change `add_reminders_batch` to return `Vec<String>` of canonical IDs in
  input order. The local `names` vec (write.rs:99-108) already has them.
- Leave `create_note` / `create_folder` as `Result<String>`.

CLI:
- `reminders add -j` → `{"ok":true,"id":"Reminder/<UUID>","title":"...","list":"..."}`.
- `reminders add -j` human → `Added: <title>` to **stdout** (not stderr —
  P1-2). On stderr, keep the `hint` line.
- `reminders add-batch -j` → `{"ok":true,"count":N,"ids":["Reminder/...",...]}`.
- `notes create -j` → `{"ok":true,"id":"<uuid>","folder":"...","title":"..."}`.
- `notes create -j` human → `Created note <title> in <folder>` to stdout.
- For `RAW:` create, return and emit the same shape — no special-case fork.

**Tests.**

- API: extend existing fixture-driven tests in
  `crates/icloud-api/tests/json_roundtrip.rs` if any exercise `modify_records`;
  otherwise add a small mock for `CloudKitClient::modify_records` in
  `reminders/write.rs` tests that returns a canned response.
- CLI: `crates/icloud-cli/tests/output_contract.rs` (new) — feed
  pre-canned reminder/note structs through the formatter functions and assert
  JSON shape and stream destination.

**Acceptance.**

```bash
icloud reminders add -l Reminders "id-smoke" -j \
  | jq -e '.id | startswith("Reminder/") and (length > 9)'
icloud reminders add-batch -l Reminders a b -j \
  | jq -e '.ids | length == 2 and all(startswith("Reminder/"))'
printf '# title\n\nbody\n' | icloud notes create --folder Notes -j \
  | jq -e '.id and .title == "title" and .folder == "Notes"'
```

---

### P1-2. Mutation success on stdout, hints/prompts on stderr

**Where.**
- `crates/icloud-cli/src/output.rs:319` (`print_hme_action`) — `eprintln!`.
- `crates/icloud-cli/src/output.rs:331` (`print_ok`) — `eprintln!`.
- `crates/icloud-cli/src/output.rs:343` (`print_ok_with`) — `eprintln!`.

The hint helper (`output.rs:88-100`) is **already correct** — it writes to
stderr and is meant for "what to do next" suggestions, which belong on
stderr. Don't change it.

**Fix.**

- `print_ok`, `print_ok_with`, and the success branch of `print_hme_action`
  → switch human-mode output from `eprintln!` to `println!`.
- The failure branch of `print_hme_action` ("…: failed") → keep on stderr;
  it's a diagnostic, not a result. The exit code still encodes failure.
- Consider documenting the contract at the top of `output.rs`:

```rust
//! Stream contract:
//!   stdout — command results, JSON payloads, success confirmations.
//!   stderr — diagnostics, progress, prompts, hints, dry-run previews.
```

**Tests** (`crates/icloud-cli/tests/output_contract.rs`):

- `print_ok_human_writes_to_stdout`
- `print_ok_with_human_writes_to_stdout`
- `print_ok_json_writes_to_stdout`
- `print_hme_action_ok_human_writes_to_stdout`
- `hint_writes_to_stderr` (regression — keep stderr behaviour locked in)

Tests use `gag::BufferRedirect` or write to a pair of `Vec<u8>` via a tiny
indirection layer; pick whichever is smaller.

**Acceptance.**

```bash
icloud reminders add -l Reminders "stream-smoke" 1>/tmp/out 2>/tmp/err
test -s /tmp/out                # stdout has the success line
! grep -q 'Added' /tmp/err      # stderr does not
grep -q 'list -' /tmp/err       # but stderr does have the hint
icloud reminders delete stream-smoke --force >/dev/null  # cleanup
```

---

### P1-3. `--plain` for whoami and HME list

**Where.**
- `crates/icloud-cli/src/output.rs:279` — `print_whoami(json: bool, ...)`.
- `crates/icloud-cli/src/cmd_hme.rs:53` — `handle_hme(json: bool, ...)`.
- `crates/icloud-cli/src/cmd_hme.rs:55-60` — `HmeCmd::List` always calls
  `print_json(&v)`, regardless of `--plain` or `--quiet`.
- `crates/icloud-cli/src/main.rs:518` — wires `handle_hme(json, ...)`.
- `crates/icloud-cli/src/main.rs:640` — wires `print_whoami(json, ...)`.

**Fix.**

- Change `print_whoami` to accept `OutputMode`. Plain row:
  `session_path\tapple_id\tstatus\tdsid` (4 fields, tab-separated, no header).
- Change `handle_hme` to accept `OutputMode`.
- Add `print_hme_list_mode(mode, &aliases)` in `output.rs`:
  - `mode.json` → `print_json(&aliases)` of the **parsed** form (use
    `HmeAlias::parse_list_response`, already in
    `crates/icloud-api/src/hme.rs:72`).
  - `mode.plain` → one TSV row per alias:
    `anonymous_id\thme\tlabel\tforward_to\tactive\torigin\tcreated_iso`.
  - `mode.quiet` → `println!("{}", aliases.len())`.
  - default human → table.

This also fixes a quiet-mode gap: `hme list -q` currently prints raw JSON.

**Tests.**

- Formatter test for plain whoami.
- HME parser/formatter tests using a redacted `crates/icloud-api/tests/`
  fixture (a captured `list_aliases` response with the user's anonymous_id
  and email scrubbed).

**Acceptance.**

```bash
icloud --plain whoami    | awk -F '\t' 'NF == 4 { ok=1 } END { exit !ok }'
icloud --plain hme list  | awk -F '\t' 'NF >= 6 { ok=1 } END { exit !ok }'
icloud --quiet hme list  | grep -qE '^[0-9]+$'
icloud --quiet whoami    | grep -qE '^(ok|failed)$'   # decide quiet shape
```

The quiet shape for whoami is a design call — `ok`/`failed` is the most
useful. Pick that and lock it in tests.

---

## Phase 2 — VFS contract cleanup

### P2-1. Hide `/Attachments` from root listing until reads are real

**Where.**
- `crates/icloud-bash/src/vfs.rs:826-832` (`readdir_with_file_types` for
  `Root`).
- `crates/icloud-bash/src/pathmap.rs:135-141` (`classify` accepts
  `/Attachments`).
- `crates/icloud-bash/src/pathmap.rs:154` (`is_icloud_prefix` accepts it).
- `crates/icloud-bash/SPEC.md` ("Filesystem Layout" section).
- `crates/icloud-cli/src/cmd_search.rs:267` already rejects it (regression
  test exists).

**Decision.** Hide from root listing. Keep the `VfsTarget::Attachments*`
variants so error mapping stays consistent and Markdown reverse-link
references (`crates/icloud-api/src/notes/markdown.rs`) still parse without
crashes — they just won't resolve.

**Fix.**

```rust
// vfs.rs:826
VfsTarget::Root => Ok(vec![
    dent("Notes", true),
    dent("Reminders", true),
    dent("HideMyEmail", true),
    dent("tmp", true),
]),
```

Stat / read / readdir of `/Attachments` returns
`FsError::NotFound { operation: "open" }` rather than the existing
`Other { message: "Attachment binary download not yet implemented" }` —
that message overpromises future support.

Add to `crates/icloud-bash/SPEC.md`:

```
Future: /Attachments/<note-id>/<filename> read-only access. Not in v1.
```

**Tests** (`tests/golden_vfs.rs`):

- `root_listing_excludes_attachments`
- `attachments_path_is_not_found_consistently` (read, stat, readdir).

---

### P2-2. Live-verify Notes folder mkdir

**Where.**
- `crates/icloud-bash/src/vfs.rs:777-783` (mkdir → `create_folder` via retry).
- `crates/icloud-api/src/notes/write.rs:451` (`create_folder` returns
  `Result<String>`).

**Status.** Source looks right. The earlier probe saw mkdir succeed but
nothing show up in `notes folders`, which suggests either:
1. `notes folders` reads from a stale cache without re-syncing first, or
2. Apple Notes silently de-duplicates folder names and the new folder
   actually isn't created server-side, or
3. The folder is created but in a parent we don't expect (Notes folders are a
   tree, see `NoteFolder::parent_id`).

**Action.** Live-verify and fix the cache visibility:

1. Run `icloud bash -c "mkdir /Notes/SmokeFolder-$$"`.
2. Run `icloud notes folders --json`. If the folder isn't there, run
   `icloud notes sync` first and retry.
3. If it shows up after a manual sync, the issue is **cache freshness** —
   `mkdir` updates the in-memory cache and the redb store, but a separate
   `icloud notes folders` invocation loads its own cache and only network-
   syncs if `max_age` says so. Change cmd_bash to **save and re-load** the
   cache with the new folder, or just call `engine.sync(false)` after
   `create_folder` returns.
4. If it doesn't show up after sync, debug at the CloudKit layer (compare
   modify_records body against ElyaConrad/iCloud-API).

**Tests** (ignored live):

- `live_notes_folder_create_visible_after_sync`

**Acceptance.**

```bash
folder="SmokeFolder-$(date +%s)"
icloud bash -c "mkdir /Notes/$folder"
icloud notes sync >/dev/null
icloud notes folders --json | jq -e --arg f "$folder" '.[] | select(.name == $f)'
```

---

### P2-3. Lock `/tmp` to passthrough classification

**Where.**
- `crates/icloud-bash/src/pathmap.rs` — already correct
  (`classify("/tmp/foo") == Passthrough` per test on line 205).

**Action.** Add tests that lock the contract in:

- `is_icloud_prefix("/tmp") == false` (already on line 214).
- After P0-2, `tmp_is_present_at_root_listing` and
  `tmp_is_writable_via_inmemory_fs` regression tests.

No code change beyond P0-2.

---

## Phase 3 — CLI semantics, help, parity

### P3-1. Align due-date help between `add` and `edit`

**Where.**
- `crates/icloud-cli/src/cmd_reminders.rs:78` (`Add::due` help — advertises
  `today`, `tomorrow`, `YYYY-MM-DD`, `YYYY-MM-DD HH:mm`).
- `crates/icloud-cli/src/cmd_reminders.rs:138` (`Edit::due` help —
  advertises only `YYYY-MM-DD`).
- `crates/icloud-cli/src/main.rs:386` (`resolve_due_date`).

**Fix.**

- Make `Edit::due` help match `Add::due`.
- Confirm `crates/icloud-api/src/title_doc.rs:295` (`str_to_ts`) accepts the
  datetime form. It currently does **not** — it parses only `%Y-%m-%d`. Either
  extend `str_to_ts` to try both formats, or downgrade the help to
  `YYYY-MM-DD` only on both sides. Pick the latter for finalization; document
  the time-of-day form as future work.
- Add unit tests for `today`, `tomorrow`, `yesterday`, fixed date,
  unknown input (passes through unchanged).

---

### P3-2. Notes folder CLI parity with Reminders Lists

**Where.**
- `crates/icloud-cli/src/cmd_reminders.rs:46-64` (`Lists` has `--rename`,
  `--delete`, `--create`).
- `crates/icloud-cli/src/cmd_notes.rs:31-43` (`Folders` has only `--delete`).
- `crates/icloud-api/src/notes/write.rs:451` (`create_folder` exists).

**Fix.**

- Add `--create` to `notes folders <name>`. Use `create_folder` wrapped in
  `with_notes_retry` (P0-3).
- `--rename` is **not** supported by the API today (no `rename_folder` in
  `notes/write.rs`). Don't expose a flag for it; file a follow-up issue
  ("Notes: rename folder via CloudKit `update` op on `Folder` record") and
  exit. Don't pad CLI surface with broken flags.

**Tests.**

- Live ignored: create folder via CLI → list folders → delete folder.
- CLI parser test: `notes folders X --create` succeeds; combination with
  `--delete` errors.

---

### P3-3. Document exit codes (and align text with source)

**Where.**
- `crates/icloud-cli/src/main.rs:29-48` defines and maps the codes.

**Source today.**

| Code | Constant | Maps from |
|---|---|---|
| 0 | (success) | — |
| 2 | `EXIT_USAGE` | `Error::Usage`, `Error::Reminders/Notes` containing `not found`/`no changes`/`missing` |
| 3 | `EXIT_AUTH`  | `Error::Auth`, `Error::Session`, `Error::Keyring` |
| 4 | `EXIT_UPSTREAM` | everything else (transport, cache, IO, transfer, generic Reminders/Notes) |

**Fix.**

- Add a `clap` `after_long_help` to the top-level `Cli`:

```text
EXIT CODES:
  0  success
  2  usage error or unknown resource
  3  authentication / session error
  4  transport, upstream, or cache error
```

- Either accept the conflation of "transport" and "cache" under code 4, or
  split: introduce `EXIT_CACHE = 5` and route `Error::Reminders/Notes`
  containing `redb`/`lock file`/`cache` to it. The split has weak motivation
  for v1; defer.
- Audit `exit_code` to make sure the matched substrings still cover today's
  errors. Add a test in `crates/icloud-cli/tests/exit_codes.rs` that
  constructs each error variant and asserts the mapped code.

---

### P3-4. `version` subcommand with JSON

**Where.**
- `crates/icloud-cli/src/main.rs:135-138` (`#[command(version)]` only).

**Fix.**

- Add a `Command::Version` variant that prints `--version` text in human mode
  and a small JSON object in JSON mode:

```json
{"name":"icloud","version":"0.1.0","commit":"6e6f623","build_date":"2026-04-27T..."}
```

- Use `vergen` only if it doesn't add a build dependency we'd otherwise avoid;
  otherwise read `CARGO_PKG_VERSION` and call `git rev-parse HEAD` from a
  `build.rs` that fails open (so vendored builds work).

**Acceptance.**

```bash
icloud version --json | jq -e '.name == "icloud" and .version'
```

---

### P3-5. Document `--body` verbatim semantics

**Where.**
- `crates/icloud-cli/src/cmd_notes.rs:62-63` (the `--body` flag).
- `crates/icloud-cli/src/main.rs:455` (`read_body_or_stdin`).

**Decision.** Keep `--body` literal. Add to long help:

```
--body <BODY>
  Markdown body, taken verbatim. Shells do not interpret escape sequences in
  -- argument values. For multi-line bodies, pipe stdin instead:
    printf '# Title\n\nBody line 1\nBody line 2\n' \
      | icloud notes create --folder Notes
```

No code change.

---

### P3-6. Honor `--no-input` in 2FA prompt

**Where.**
- `crates/icloud-cli/src/main.rs:364-382` (`prompt_2fa`).

**Bug.** `prompt_2fa` ignores the global `--no-input` flag. If `ICLOUD_2FA_CODE`
isn't set and `--no-input` is, we still block on `read_line` from stdin.

**Fix.** Plumb `out.no_input` (already plumbed everywhere else) into
`prompt_2fa` and return `Err(Error::Auth("2FA required, --no-input set"))`
immediately when set. Combined with `ICLOUD_2FA_CODE`, this gives clean
non-interactive automation.

---

### P3-7. Add `--code-stdin` mirror to `--password-stdin`

**Where.**
- `crates/icloud-cli/src/main.rs:200-215` (`Login`).

**Fix.** Add `--code-stdin` (mutually exclusive with `--code`). Same treatment
as `password_stdin` — read first line of stdin. Useful for piping output of
a TOTP tool: `oathtool ... | icloud login --username x --password-stdin`
won't work because both want stdin; users with HW tokens can paste codes via
`echo 123456 | icloud login ... --code-stdin`.

Lower priority. File as P3.

---

## Phase 4 — Bulk and ergonomics (post-v1)

These do not block the contract fixes. Track in issues, not in this document.

### P4-1. JSONL reminder import

`icloud reminders add-batch --from-jsonl <path|->`. Each line:

```json
{"title":"Buy milk","due":"tomorrow","priority":"high","notes":"...","parent":"..."}
```

Stream in, return `{"ok":true,"count":N,"items":[{"id":"...","title":"..."}]}`.
On a row failure, return non-zero with the line number in the error.

### P4-2. List-ID disambiguation

Add `--list-id List/<UUID>` everywhere `--list <name>` is accepted on
mutations, for cases with duplicate display names. `--list` stays as the
ergonomic default.

### P4-3. Bashbox glob expansion via `FileSystem::readdir`

Pathname expansion in bashbox (`shell/glob_expander.rs` already exists per
`SyncFsAdapter::glob`) needs to be wired into `expand_word_with_glob`
calls in `execute_simple_command`. Currently `echo /Notes/Drafts/*` returns
the literal pattern. Lower priority than P0-4 because broken globs are
visible (the literal `*` shows up); broken redirects silently lose data.

### P4-4. `icloud cp` glob expansion

Either expand `icloud:/Notes/Drafts/*.md` inside `cmd_cp.rs` against the VFS,
or document that recursive directory copy is the supported bulk form. Pick
the doc-only path for v1.

### P4-5. HME `reactivate`

`crates/icloud-api/src/hme.rs` has `deactivate` but no inverse. Apple's web
client supports reactivation; check the network trace and add the
corresponding API + CLI subcommand.

### P4-6. Reminders `move` between lists

CLI today has no way to move a reminder between lists without delete+recreate.
The API path is "edit reminder, change `List` field". Add `--list <name>`
to `reminders edit` for this.

---

## Cross-cutting test plan

### Unit tests (no live iCloud)

- `crates/icloud-api/src/store.rs::tests` — `find_item` matrix (P0-1).
- `crates/icloud-api/src/retry.rs::tests` — `is_cloudkit_retryable` strings.
- `crates/icloud-cli/tests/output_contract.rs` (new) — formatter shape +
  stream destination per mode (P1-1, P1-2, P1-3).
- `crates/icloud-cli/tests/exit_codes.rs` (new) — error → exit-code mapping.
- `crates/icloud-bash/tests/golden_vfs.rs` — `/tmp` lifecycle, root listing
  excludes `/Attachments`, classification preserves `/tmp` as Passthrough.

### Integration tests (no live iCloud)

- After P0-4 lands and rev is bumped: redirect tests through `Bash::exec`
  using `InMemoryFs` only (`>`, `>>`, `2>`, `&>`, pipeline-trailing redirect).

### Live ignored tests

`crates/icloud-cli/tests/live_smoke.rs`, gated on:
- `ICLOUD_CLI_LIVE=1`
- `ICLOUD_CLI_REMINDERS_LIST=Reminders` (default value)
- `ICLOUD_CLI_NOTES_FOLDER=Notes`     (default value)

Scenarios:
- Reminder add → edit → complete → delete using returned JSON id (P0-1, P1-1).
- Note create from stdin → get → delete using returned JSON id (P1-1).
- VFS `/tmp` read/write (P0-2, P0-4).
- Notes folder mkdir verification (P2-2).
- HME list only — never call `generate` from automated tests
  (rate-limited per `AGENTS.md` line 70).

---

## Implementation order (locked)

1. **P0-1** — canonical ID matching. One file (`store.rs`), small change,
   unblocks every JSON-driven script.
2. **P1-1** — created IDs in JSON output. Required so the live smoke tests
   for P0-1 actually have an id to feed back.
3. **P1-2** — stdout/stderr contract. Mechanical s/`eprintln!`/`println!`/
   in three functions plus formatter tests.
4. **P0-3** — retry wrappers. 19 callsites total. Mechanical but
   error-prone — do it after the API return changes (P1-1) so closure bodies
   match.
5. **P0-2** — pre-seed `/tmp`. Tiny change in `ICloudFs::new`; touches two
   call sites in `cmd_bash.rs` and `cmd_cp.rs`.
6. **P2-1** — hide `/Attachments`. Two-line change plus SPEC update.
7. **P2-2** — live-verify Notes folder mkdir; only fix code if live test
   reveals a bug.
8. **P1-3** — `--plain` / `--quiet` for whoami and HME.
9. **P3-1, P3-2, P3-3, P3-5, P3-6** — help, parity, exit-code text. Bundle.
10. **P3-4** — `version` subcommand.
11. **P0-4** — bashbox redirect wiring. **Last** because it spans another
    repo and gates the redirect acceptance tests, but it can proceed in
    parallel with everything above as long as the rev bump lands together
    with the local consumer test.
12. **Phase 4** — separate issues, not part of "v1".

---

## Release gate

Before tagging:

```bash
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo test -p icloud-cli --test output_contract
cargo test -p icloud-cli --test exit_codes
cargo test -p icloud-bash
```

Live smoke (one disposable folder/list/account):

```bash
# P0-1, P1-1, P1-2 round-trip
id=$(icloud reminders add -l Reminders "release-gate" -j | jq -r .id)
test "$id" != null && [[ "$id" == Reminder/* ]]
icloud reminders edit  "$id" --priority high
icloud reminders complete "$id"
icloud reminders delete "$id" --force

# Notes round-trip
nid=$(printf '# release-gate\n\nbody\n' \
        | icloud notes create --folder Notes -j | jq -r .id)
test "$nid" != null
icloud notes get "$nid" >/dev/null
icloud notes delete "$nid"

# Folder mkdir
folder="GateFolder-$(date +%s)"
icloud bash -c "mkdir /Notes/$folder"
icloud notes folders --json | jq -e --arg f "$folder" '.[] | select(.name == $f)' >/dev/null

# Redirects (post-P0-4)
icloud bash -c 'echo data > /tmp/x && cat /tmp/x' | grep -qx data
icloud bash -c 'echo data > /Notes/Drafts/redirect-smoke.md' \
  && icloud notes get redirect-smoke | grep -q data \
  && icloud notes delete redirect-smoke

# Stream contract
icloud reminders add -l Reminders "stream-gate" 1>/tmp/o 2>/tmp/e
test -s /tmp/o && ! grep -q 'Added' /tmp/e
icloud reminders delete stream-gate --force >/dev/null
```

All must pass before the iCloud-CLI Claude skill (`./skills/icloud-cli`) gets
written, because the skill documents the surface — and we want that surface
to be the post-fix one, not today's.

---

## Definition of done

- All Phase 0 and Phase 1 items implemented and tested.
- `IMPROVEMENT.md`, `PROGRESS.md`, and `crates/icloud-bash/SPEC.md` agree on
  the VFS layout (no `/Attachments` in root listings, `/tmp` real, redirects
  live).
- No command path emits cookies, tokens, passwords, or session paths in
  tracing logs.
- Every changed user-facing behavior has either a unit/integration test or
  an ignored live smoke test with the exact reproduction commands listed.
- The release-gate live smoke passes once on a disposable account.
- Phase 4 items are filed as separate GitHub issues with this document
  cross-referenced.
