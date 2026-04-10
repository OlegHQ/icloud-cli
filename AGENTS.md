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

- `crates/icloud-api`: HTTP client, serde types, errors (`thiserror`), Reminders / Notes / Hide My Email clients, sync engines, cache persistence.
- `crates/icloud-cli`: `clap` commands, config path resolution, human vs JSON rendering, `icloud bash` session wiring.
- `crates/icloud-bash`: VFS (`ICloudFs`), path mapping, frontmatter, filename sanitization; depends on `icloud-api` and **bashbox** only.

## Fragility policy

- When Apple changes behavior, fix in **`icloud-api`** with clear errors (`endpoint changed`, `missing cookie`, `session expired`, etc.).
- Add **tests** with redacted JSON fixtures where struct parsing is stable.

## “Fully featured” Notes

- **v1** targets folder/list, read, create, update plain or minimally formatted body, delete. Rich text, attachments, collaboration, and password-protected notes are **later** phases and may require capability flags.
- **Current CLI:** `icloud notes list` performs a **zone sync page** (metadata + encrypted fields as returned by the API). Decryption, protobuf `Document` decode, and mutating calls are **not** wired yet; use ElyaConrad/iCloud-API and `references/notes.md` when extending.

## References

- See `references/README.md` for upstream repositories and what to port or compare against.

## Progress tracking

- Maintain a repo-root `PROGRESS.md` as the running handoff log for cross-session work.
- After any substantial refactor, bashbox integration change, or `icloud bash` / VFS architecture change, update `PROGRESS.md` with:
  - what changed
  - what was verified
  - what remains broken or risky
  - the next concrete tasks for the following session
- New sessions should read `PROGRESS.md` before planning more cleanup.

## Hide My Email rate limits

Unofficial clients report roughly **~5 generated addresses per 30 minutes** per account (family pooling may apply) and a **~700** total-alias ceiling. Surface these constraints in user-facing docs/help when expanding HME features.

## Bash / Virtual Filesystem Feature

### Goal

`icloud bash -c "COMMAND"` (and optional script path) exposes iCloud data as a POSIX virtual filesystem for shell-style exploration and scripting. The embedded interpreter is **[bashbox](https://github.com/OlegHQ/bashbox)** (Rust bash implementation with a pluggable `FileSystem` trait), pinned as a git dependency in `crates/icloud-cli/Cargo.toml` and `crates/icloud-bash/Cargo.toml`.

### Specification

Full layout and operation mapping: `crates/icloud-bash/SPEC.md`. The spec’s concurrency notes still apply at the CloudKit / cache layer even though the interpreter crate is now bashbox.

### Filesystem Layout

```
/
├── Notes/<Folder>/<Title>.md     # Markdown + YAML frontmatter
├── Reminders/<List>/<Title>.md   # Markdown + YAML frontmatter
├── HideMyEmail/aliases.json      # Read-only JSON
└── tmp/                          # Per-session scratch (in-memory)
```

### Crate structure

```
crates/icloud-bash/
├── Cargo.toml          # icloud-api + bashbox (git)
├── SPEC.md             # VFS layout, operations, concurrency
├── src/
│   ├── lib.rs
│   ├── vfs.rs          # ICloudFs — implements bashbox::fs::FileSystem
│   ├── frontmatter.rs
│   ├── pathmap.rs
│   └── sanitize.rs
└── tests/
    └── golden_vfs.rs   # smoke tests: paths, frontmatter, trivial Bash on InMemoryFs (5 tests; 1 ignored placeholder)
```

### Implementation status (summary)

- **`ICloudFs`** in `vfs.rs` composes `NotesSyncEngine`, `SyncEngine`, `HideMyEmailClient`, and `bashbox::InMemoryFs` (for `/tmp` and passthrough paths). Read/write/delete/mkdir/mv/cp and related `FileSystem` methods are implemented for iCloud paths where the API supports them; see `SPEC.md` for the intended matrix.
- **CLI** (`icloud-cli`): loads session, opens engines, builds `Bash::new(BashOptions { fs: Some(arc_icloud_fs), ... })`, runs `-c` or script input. Sets `HOME=/`, `USER=icloud`, `ICLOUD_NOTES_COUNT`, `ICLOUD_REMINDERS_COUNT`, and `ICLOUD_SESSION`.
- **Retries / conflict handling** and **multi-process stress tests** remain areas to harden in `icloud-api` / CLI usage; `PROGRESS.md` should track concrete follow-ups.

### Concurrency model (unchanged intent)

**Daemonless.** Each `icloud bash` process loads cache, syncs if needed, and talks to CloudKit on writes; redb locking and merge behavior live in `icloud-api`. For the full narrative, see the “Concurrency Model” section in `crates/icloud-bash/SPEC.md`.

### Module boundaries (bash stack)

- **`icloud-api`**: CloudKit protocol, sync engines, cache, persistence. No knowledge of bash or VFS paths.
- **`icloud-bash`**: `FileSystem` implementation and iCloud path semantics only.
- **`icloud-cli`**: User-facing `icloud bash` command and wiring.
- **`bashbox`**: Upstream interpreter — we maintain the fork at [`OlegHQ/bashbox`](https://github.com/OlegHQ/bashbox), pinned by git rev in both `icloud-cli/Cargo.toml` and `icloud-bash/Cargo.toml`. **For iCloud-specific behavior**, extend `icloud-bash` / `icloud-cli` — do not patch bashbox. **For generic bash bugs** (quoting, expansion, builtins, glob, etc.), fix them in the local checkout at `../bashbox`, run `cargo test --lib` there, commit + push, then bump the `rev = "..."` in both `Cargo.toml` files in lockstep and re-run `cargo build` at the workspace root so `Cargo.lock` updates.

### Testing strategy

- **Unit tests** in `icloud-bash` modules (`sanitize`, `frontmatter`, `pathmap`, `vfs` as applicable).
- **`tests/golden_vfs.rs`**: lightweight integration checks (path normalization, classification, frontmatter roundtrip, trivial `Bash` run on `InMemoryFs`).
- **No live CloudKit in CI** — use fixtures in `icloud-api` where present; exercise real accounts manually.

### Security

- No arbitrary process `exec` from the embedded shell; behavior is defined by bashbox’s restricted/builtin surface (configure or document allow-lists there).
- Treat session paths and caches as secrets (`AGENTS.md` security section).
- Path traversal stays within the VFS root; large writes are capped (1MB per iCloud write in `vfs.rs`).
