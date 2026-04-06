# Agent instructions: icloud-cli

This repository is a **Rust** command-line tool for **iCloud** features that matter to automation: **Reminders**, **Notes**, and **Hide My Email**. It talks to the same **private web / CloudKit-style** endpoints that **icloud.com** uses. There is **no** dependency on macOS **EventKit** or other Apple-only client frameworks.

## Project goals

- Provide one CLI that humans and scripts (including LLM agents) can use reliably.
- Prefer **session reuse** over repeated full logins; support **2FA** explicitly.
- Keep **protocol and types** in the `icloud-api` library crate; keep **argument parsing and output** in the `icloud-cli` binary crate.

## Non-goals

- Circumventing Apple account security, captcha bypass, or bulk abuse of iCloud services.
- Storing Apple IDs/passwords in the repository or in committed config.
- Promising binary compatibility if Apple changes private endpoints (they sometimes do).
- **On-disk format migrations** or backwards compatibility for older caches (for example legacy JSON reminder caches or pre-split session files). Users are expected to **re-login** and **re-sync** when formats change.

## Security and privacy

- Session files live under the XDG config directory (or platform equivalent). Restrict permissions on disk; treat session files like credentials.
- **Never** log raw cookies, CloudKit tokens, webservice tokens, or passwords. Use structured tracing at `debug` only for redacted metadata (e.g. HTTP status, endpoint path).
- Unofficial clients may break or attract account scrutiny. Mirrors the disclaimers in community libraries listed under `references/`.

## CLI conventions

- **Resource-oriented** subcommands: `list`, `get`, `create`, `update`, `delete`, and service-specific verbs such as `complete` for reminders.
- **Global `--json`**: stdout emits JSON suitable for scripts; human mode uses plain text/tables. Diagnostics go to **stderr**.
- **Stable exit codes**: `0` success; non-zero for usage errors, auth errors, transport errors, and domain errors (see `icloud-cli` implementation).
- Prefer **flags** over positional arguments except for resource IDs where natural.

## Authentication model

- **Login** establishes a persisted **session** (cookies + service tokens as required per domain).
- **2FA**: When Apple requires a trust code, the CLI may prompt interactively or accept a code from environment/flag only where explicitly documented (automation must be opt-in and obvious).
- Reuse sessions aggressively to avoid **login storms** (same guidance as ElyaConrad/iCloud-API and pyicloud-style clients).

## Module boundaries

- `crates/icloud-api`: HTTP client, serde types, errors (`thiserror`), Reminders / Notes / Hide My Email clients.
- `crates/icloud-cli`: `clap` commands, config path resolution, human vs JSON rendering.

## Fragility policy

- When Apple changes behavior, fix in **`icloud-api`** with clear errors (`endpoint changed`, `missing cookie`, `session expired`, etc.).
- Add **tests** with redacted JSON fixtures where struct parsing is stable.

## “Fully featured” Notes

- **v1** targets folder/list, read, create, update plain or minimally formatted body, delete. Rich text, attachments, collaboration, and password-protected notes are **later** phases and may require capability flags.
- **Current CLI:** `icloud notes list` performs a **zone sync page** (metadata + encrypted fields as returned by the API). Decryption, protobuf `Document` decode, and mutating calls are **not** wired yet; use ElyaConrad/iCloud-API and `references/notes.md` when extending.

## References

- See `references/README.md` for upstream repositories and what to port or compare against.

## Hide My Email rate limits

Unofficial clients report roughly **~5 generated addresses per 30 minutes** per account (family pooling may apply) and a **~700** total-alias ceiling. Surface these constraints in user-facing docs/help when expanding HME features.

## Bash / Virtual Filesystem Feature

### Goal

`icloud bash -c "COMMAND"` exposes iCloud data as a POSIX virtual filesystem that LLM agents can traverse with standard shell commands. Powered by [just-bash](https://github.com/arthur-zhang/just-bash) (pure-Rust bash interpreter with pluggable `FileSystem` trait).

### Specification

Full spec lives in `crates/icloud-bash/SPEC.md`. Golden test suite: `crates/icloud-bash/tests/golden_vfs.rs` (126 tests).

### Filesystem Layout

```
/
├── Notes/<Folder>/<Title>.md     # Markdown + YAML frontmatter
├── Reminders/<List>/<Title>.md   # Markdown + YAML frontmatter
├── HideMyEmail/aliases.json      # Read-only JSON
└── tmp/                          # Per-session scratch (in-memory)
```

### Crate Structure

```
crates/icloud-bash/
├── Cargo.toml          # depends on icloud-api + just-bash
├── SPEC.md             # Full specification (filesystem layout, operations, concurrency)
├── src/
│   ├── lib.rs
│   ├── vfs.rs          # ICloudFs — implements just_bash::fs::FileSystem
│   ├── frontmatter.rs  # YAML frontmatter parse/render for notes + reminders
│   ├── pathmap.rs      # VFS path ↔ CloudKit record ID resolution
│   └── sanitize.rs     # Filename ↔ title conversion (slash, collision, truncation)
└── tests/
    └── golden_vfs.rs   # 126 golden tests (spec-driven)
```

### Implementation Plan

**Phase 1 — Path resolution + read-only VFS** (no CloudKit writes)

1. **`sanitize.rs`**: Implement `title_to_filename()` and `filename_to_title()`. Handle `/` → `∕`, collision suffixes ` (2)`, truncation to 255 bytes. Unit tests.
2. **`frontmatter.rs`**: Implement `render_note(NoteData, body) → String` and `render_reminder(ReminderData) → String`. Parse writable reminder frontmatter on write. Unit tests.
3. **`pathmap.rs`**: Route VFS paths to the correct service + record. Parse `/Notes/<folder>/<file>.md` into `(Service::Notes, folder_id, record_id)`. Handle `/tmp/`, `/HideMyEmail/`, root. Classify each path as `File | Dir | NotFound | Invalid`. Unit tests.
4. **`vfs.rs`**: Implement `ICloudFs` struct holding `NotesSyncEngine`, `SyncEngine`, `HideMyEmailClient`, and `InMemoryFs` (for `/tmp/`). Implement read-only `FileSystem` methods: `read_file`, `readdir`, `readdir_with_file_types`, `stat`, `lstat`, `exists`, `realpath`, `resolve_path`, `get_all_paths`. Wire up the 22-method trait; return `EROFS` / `EACCES` for unimplemented writes.
5. **Wire into CLI**: Add `icloud bash -c "COMMAND"` subcommand to `icloud-cli`. Load session, sync engines, create `ICloudFs`, pass to `just_bash::Bash::new(fs)`, execute, print output.

**Phase 2 — Write operations**

6. **`write_file`**: Parse path → service. For notes: strip frontmatter from input, call `create_note` (new) or `update_note` (existing). For reminders: parse frontmatter for writable fields, call `add_reminder` (new) or `edit_reminder` (existing). For `/tmp/`: delegate to `InMemoryFs`.
7. **`append_file`**: Read current content, append, write back (notes/reminders). Delegate to `InMemoryFs` for `/tmp/`.
8. **`rm`**: Map to `delete_note` / `delete_reminder`. Handle `recursive` for directories (delete all items then folder/list).
9. **`mkdir`**: Map to reminder list creation (`create_list`). For notes folders: create via placeholder or cache manipulation.
10. **`mv`**: Within-service same folder = rename (update title). Cross-folder same service = `move_note` or error. Cross-service = `EXDEV`.
11. **`cp`**: Read source, create at destination. Allow `/tmp/` as neutral ground between services.
12. **`chmod`**, **`symlink`**, **`link`**, **`utimes`**: `chmod` = no-op on iCloud, real on `/tmp/`. `symlink` only in `/tmp/`. `link` = `EACCES` everywhere. `utimes` = no-op on iCloud.

**Phase 3 — Concurrency + conflict handling**

13. **Shared read lock on cache load**: Change `RedbStore::load_cache` to use `lock_shared()` instead of `lock_exclusive()`.
14. **Save-phase merge**: On save, detect sync token divergence. If another process advanced the token, write only our dirty items (don't overwrite their sync progress).
15. **CloudKit conflict retry**: Wrap write operations in `write_with_retry` (detect stale `recordChangeTag`, re-sync, retry up to 3 times). Map `RECORD_NOT_FOUND` to `ENOENT`.
16. **Integration test**: Multi-process test spawning 5 concurrent `icloud bash` processes doing reads + writes. Verify no data loss, no deadlocks.

**Phase 4 — Polish**

17. **Environment variables**: Set `HOME=/`, `USER=icloud`, `ICLOUD_NOTES_COUNT`, `ICLOUD_REMINDERS_COUNT` in bash environment.
18. **Error messages**: Map all CloudKit errors to POSIX errors with helpful messages (session expired → `EIO` with hint).
19. **Script file support**: `icloud bash script.sh` reads and executes a file.
20. **Size limits**: Reject writes > 1MB for notes/reminders. No limit for `/tmp/`.

### Concurrency Model

**Daemonless**. Each `icloud bash` process is self-contained:

- **Load**: shared read lock on redb (~1ms), then release. Sync from CloudKit if stale (no lock held).
- **Operate**: all reads from in-memory cache (zero I/O). Writes go to CloudKit directly (optimistic — `recordChangeTag` as version).
- **Save**: exclusive lock on redb only for the write transaction (~1ms). Merge with other processes' changes via sync token comparison.

CloudKit is the conflict arbiter. Two processes writing different records: no contention. Same record: loser re-syncs and retries (3 attempts). See `SPEC.md § Concurrency Model` for full details.

### Module Boundaries

- **`icloud-api`**: All CloudKit protocol, sync engines, cache, persistence. No bash/VFS knowledge.
- **`icloud-bash`**: VFS implementation (`FileSystem` trait), frontmatter, path mapping, filename sanitization. Depends on `icloud-api` for engines and `just-bash` for the bash interpreter.
- **`icloud-cli`**: CLI entry point for `icloud bash` subcommand. Wires session loading, engine creation, and VFS together.

### Testing Strategy

- **Unit tests** in each `icloud-bash` module (sanitize, frontmatter, pathmap)
- **Golden test suite** (`golden_vfs.rs`): 126 tests covering every operation, error, edge case, and concurrency scenario against mock backends
- **Integration tests**: Full `just-bash` execution against `ICloudFs` with mock CloudKit responses
- **No real CloudKit calls in tests** — all mocked. Real integration tested manually.

### Security

- No shell escape — all commands route through `just-bash` command registry (70+ builtins, no real exec)
- No network access from bash (curl/wget disabled or allow-listed)
- No access outside VFS (`/etc/passwd` → `ENOENT`)
- Path traversal blocked (`.` / `..` resolved within VFS)
- Execution limits: 100K commands, 1M iterations, 1K recursion depth per session
- File size limit: 1MB per iCloud write
