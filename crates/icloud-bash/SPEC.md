# icloud-bash: Virtual Filesystem Specification

## Overview

`icloud-bash` implements the **bashbox** `FileSystem` trait to expose iCloud
Notes, Reminders, and Hide My Email as a POSIX-like virtual filesystem.  Agents
run shell commands against this VFS; every legal file operation maps to a
CloudKit API call.  Illegal operations return appropriate POSIX errors.

## Filesystem Layout

```
/
├── Notes/                              # iCloud Notes
│   ├── <FolderName>/                   # One directory per Notes folder
│   │   ├── <NoteTitle>.md              # Markdown file with YAML frontmatter
│   │   └── ...
│   └── Notes/                          # Default folder (always exists)
│       └── ...
├── Reminders/                          # iCloud Reminders
│   ├── <ListName>/                     # One directory per reminder list
│   │   ├── <ReminderTitle>.md          # Markdown file with YAML frontmatter
│   │   └── ...
│   └── ...
├── HideMyEmail/                        # Hide My Email (read-only)
│   └── aliases.json                    # JSON array of all aliases
└── tmp/                                # Scratch space (in-memory, no sync)
```

### Path Anatomy

| Path pattern | Maps to | Mutable |
|---|---|---|
| `/` | Root listing | No |
| `/Notes/` | List note folders | Read-only dir |
| `/Notes/<Folder>/` | List notes in folder | Create/delete files |
| `/Notes/<Folder>/<Title>.md` | Single note | Read/write/delete |
| `/Reminders/` | List reminder lists | Read-only dir |
| `/Reminders/<List>/` | List reminders | Create/delete files |
| `/Reminders/<List>/<Title>.md` | Single reminder | Read/write/delete |
| `/HideMyEmail/` | Service root | Read-only |
| `/HideMyEmail/aliases.json` | All aliases | Read-only |
| `/tmp/` | Temporary storage | Full read/write |
| `/tmp/**` | Arbitrary files | Full read/write |

## File Formats

### Note File (`/Notes/<Folder>/<Title>.md`)

```markdown
---
id: "a1b2c3d4-e5f6-..."
folder: "Work"
modified: "2024-01-15 10:30"
---
# My Note Title

Body text in Markdown.

- Lists work
- **Bold** and *italic* too
```

**Frontmatter fields** (read-only unless noted):

| Field | Type | Writable | Description |
|---|---|---|---|
| `id` | string | no | CloudKit record name |
| `folder` | string | no | Folder display name (use `mv` to change) |
| `modified` | string | no | Last modification timestamp |

On **write**, only the Markdown body (everything after the `---` closing
delimiter) is sent to the API.  Frontmatter is regenerated from the API
response.  If a write changes the first line (`# Title`), the filename is
updated to match.

### Reminder File (`/Reminders/<List>/<Title>.md`)

```markdown
---
id: "Reminder/A1B2C3D4-..."
list: "Shopping"
completed: false
due: "2024-01-20"
priority: "none"
notes: ""
---
Buy groceries for the week
```

**Frontmatter fields**:

| Field | Type | Writable | Default | Description |
|---|---|---|---|---|
| `id` | string | no | — | CloudKit record name |
| `list` | string | no | — | List name (use `mv` to change) |
| `completed` | bool | **yes** | false | Completion status |
| `due` | string? | **yes** | null | Due date (YYYY-MM-DD or YYYY-MM-DD HH:mm) |
| `priority` | string | **yes** | "none" | high, medium, low, none |
| `notes` | string | **yes** | "" | Notes text |

On **write**, writable frontmatter fields are parsed and applied via
`edit_reminder()`.  The body text (after frontmatter) becomes the reminder
title.  Changing the body text triggers a title rename.

### HME Aliases File (`/HideMyEmail/aliases.json`)

```json
[
  {
    "anonymous_id": "...",
    "email": "random@icloud.com",
    "label": "Shopping",
    "note": "",
    "is_active": true,
    "forward_to": "user@example.com",
    "created_at": "2024-01-01"
  }
]
```

This file is **read-only**.  Use `icloud hme` CLI commands for mutations.

## Operation Mapping

### File Read (`read_file`, `cat`)

| Path | Action |
|---|---|
| `/Notes/<F>/<T>.md` | `engine.fetch_body(id)` → render frontmatter + markdown |
| `/Reminders/<L>/<T>.md` | Read from cache → render frontmatter + body |
| `/HideMyEmail/aliases.json` | `hme.list_aliases()` → format JSON |
| `/tmp/**` | Read from in-memory store |
| Other | `ENOENT` |

### File Write (`write_file`, redirect `>`)

| Path | Action |
|---|---|
| `/Notes/<F>/<T>.md` (new) | `engine.create_note(md, folder)` |
| `/Notes/<F>/<T>.md` (existing) | `engine.update_note(id, md)` |
| `/Reminders/<L>/<T>.md` (new) | `engine.add_reminder(title, list, ...)` |
| `/Reminders/<L>/<T>.md` (existing) | `engine.edit_reminder(id, ...)` |
| `/HideMyEmail/**` | `EROFS` (read-only filesystem) |
| `/tmp/**` | Write to in-memory store |
| `/Notes/` or `/Reminders/` (dir) | `EISDIR` |
| `/` | `EACCES` |

### File Delete (`rm`)

| Path | Action |
|---|---|
| `/Notes/<F>/<T>.md` | `engine.delete_note(id)` |
| `/Reminders/<L>/<T>.md` | `engine.delete_reminder(id)` |
| `/HideMyEmail/**` | `EROFS` |
| `/tmp/**` | Remove from in-memory store |
| Any directory | `EISDIR` (use `rmdir`) |

### Directory Read (`readdir`, `ls`)

| Path | Action |
|---|---|
| `/` | Return `["Notes", "Reminders", "HideMyEmail", "tmp"]` |
| `/Notes/` | `engine.get_folders()` → folder names |
| `/Notes/<F>/` | `engine.get_notes()` filtered by folder → filenames |
| `/Reminders/` | `engine.get_lists()` → list names |
| `/Reminders/<L>/` | `engine.get_reminders()` filtered by list → filenames |
| `/HideMyEmail/` | `["aliases.json"]` |
| `/tmp/` | List in-memory entries |
| `/tmp/**/` | List in-memory entries recursively |

### Directory Create (`mkdir`)

| Path | Action |
|---|---|
| `/Notes/<NewFolder>` | Create note folder (via creating a placeholder note and folder) |
| `/Reminders/<NewList>` | `engine.create_list(name)` |
| `/tmp/**` | Create in in-memory store |
| `/HideMyEmail/**` | `EROFS` |
| `/` | `EEXIST` |
| Nested under existing file | `ENOTDIR` |

### Directory Delete (`rmdir`)

| Path | Action |
|---|---|
| `/Notes/<F>` | Delete all notes in folder, then remove folder from cache |
| `/Reminders/<L>` | `engine.delete_list(name)` |
| `/tmp/**` | Remove from in-memory store |
| `/`, `/Notes/`, `/Reminders/` | `EACCES` (protected) |
| `/HideMyEmail/` | `EROFS` |
| Non-empty directory (without `-r`) | `ENOTEMPTY` |

### File Move/Rename (`mv`)

| Source | Destination | Action |
|---|---|---|
| `/Notes/<F1>/<T>.md` | `/Notes/<F2>/<T>.md` | `engine.move_note(id, new_folder)` |
| `/Notes/<F>/<T1>.md` | `/Notes/<F>/<T2>.md` | `engine.update_note(id, new_title_md)` |
| `/Reminders/<L>/<T>.md` | `/Reminders/<L>/<T2>.md` | `engine.edit_reminder(id, title: new_title)` |
| `/tmp/**` | `/tmp/**` | In-memory move |
| Cross-service move | — | `EXDEV` (cross-device link) |
| Into `/HideMyEmail/` | — | `EROFS` |

### File Copy (`cp`)

| Source | Destination | Action |
|---|---|---|
| `/Notes/<F>/<T>.md` | `/Notes/<F2>/<T>.md` | Read source, `engine.create_note(body, dest_folder)` |
| `/tmp/**` → `/Notes/**` | | Read tmp file, create note |
| `/Notes/**` → `/tmp/**` | | Read note, write to tmp |
| Into `/HideMyEmail/` | — | `EROFS` |
| Cross-service | — | `EXDEV` |

### Stat (`stat`)

Returns `FsStat` with:

| Field | Notes/Reminders files | Directories | `/tmp` files |
|---|---|---|---|
| `is_file` | true | false | true |
| `is_dir` | false | true | depends |
| `size` | byte length of rendered content | 0 | byte length |
| `mode` | `0o644` | `0o755` | `0o644` |
| `mtime` | `modified_ts` from cache | 0 | write time |

### Other Operations

| Operation | Behavior |
|---|---|
| `chmod` | No-op (always returns Ok) |
| `symlink` | Only in `/tmp/` — `EACCES` elsewhere |
| `ln` (hard link) | `EACCES` everywhere (not meaningful for CloudKit) |
| `touch` | On existing file: no-op. On new file: create empty note/reminder. In `/tmp/`: create empty file. |
| `pwd` | Returns current virtual working directory |
| `cd` | Navigate within the VFS |

## Filename Sanitization

Filenames are derived from titles.  The following rules apply:

1. Replace `/` with `∕` (U+2215 DIVISION SLASH)
2. Replace `\0` with empty string
3. Collapse consecutive spaces to single space
4. Trim leading/trailing whitespace
5. Truncate to 255 bytes on a char boundary
6. If empty after sanitization, use `Untitled`
7. Append `.md` suffix
8. On collision (same folder), append ` (2)`, ` (3)`, etc.

Reverse mapping (filename → title): strip `.md`, reverse U+2215 → `/`.

## Error Mapping

| Condition | POSIX errno | Message |
|---|---|---|
| Path does not exist | `ENOENT` | No such file or directory |
| Write to read-only area | `EROFS` | Read-only file system |
| Permission denied | `EACCES` | Permission denied |
| Is a directory | `EISDIR` | Is a directory |
| Not a directory | `ENOTDIR` | Not a directory |
| Directory not empty | `ENOTEMPTY` | Directory not empty |
| Cross-service move | `EXDEV` | Invalid cross-device link |
| File already exists | `EEXIST` | File exists |
| CloudKit API error | `EIO` | Input/output error: {detail} |
| Session expired | `EIO` | Session expired — run `icloud login` |
| Invalid frontmatter | `EINVAL` | Invalid argument: {detail} |

## Sync Model

1. **On filesystem mount** (bash session start): Run `engine.sync(false)` for
   Notes and Reminders to populate caches.
2. **Reads** are served from cache (no network call) except `fetch_body` which
   fetches the full note body on demand.
3. **Writes** call the CloudKit API immediately (create/update/delete) and
   update the local cache on success.
4. **On filesystem unmount** (bash session end): Call `store.save_cache()` to
   persist the updated cache to redb.
5. **No background sync** during a session — the VFS is a snapshot with
   write-through.

## Security Constraints

1. **No shell escape**: Commands cannot exec real binaries. All commands route
   through bashbox's command registry.
2. **No network access** from bash (curl/wget disabled or limited to allow-list).
3. **No access outside VFS**: Paths like `/etc/passwd`, `/home/...` return `ENOENT`.
4. **Execution limits**: Max 100,000 commands, 1,000,000 iterations, 1,000
   recursion depth per session.
5. **File size limit**: 1 MB per file write.  Notes API has practical limits.
6. **Rate limiting**: Write operations are throttled to avoid CloudKit rate
   limits (especially HME: ~5 per 30 min).

## Environment Variables

| Variable | Value | Description |
|---|---|---|
| `HOME` | `/` | Home directory |
| `USER` | `icloud` | Virtual user |
| `SHELL` | `/bin/bash` | Shell path (virtual) |
| `PWD` | `/` (initial) | Working directory |
| `ICLOUD_SESSION` | `<path>` | Session file path (for reference) |
| `ICLOUD_NOTES_COUNT` | `<n>` | Number of synced notes |
| `ICLOUD_REMINDERS_COUNT` | `<n>` | Number of synced reminders |

## CLI Integration

```
icloud bash [SCRIPT]           # Run a bash script against the VFS
icloud bash -c "COMMAND"       # Run a single command
icloud bash                    # Interactive mode (future)
```

Global flags (`--json`, `--session`, `--secrets`, etc.) apply.  The bash
session inherits the authenticated iCloud session.

## Concurrency Model

### Problem Statement

Multiple LLM agents call `icloud bash -c "..."` concurrently against the same
iCloud account.  Each invocation is a separate OS process.  We must handle:

1. **Cache coherence** — two processes reading/writing the same redb store
2. **CloudKit conflicts** — two writers racing on the same record's `recordChangeTag`
3. **Sync token divergence** — processes advancing the sync token independently
4. **Rate pressure** — N concurrent agents × M writes = N×M API calls
5. **Session sharing** — all processes share one authenticated session

### Design Principle: No Daemon Required

The primary architecture is **daemonless**: every `icloud bash` invocation is a
self-contained process that loads state, operates, and saves.  Concurrency is
handled through **optimistic concurrency control** on CloudKit and
**fine-grained file locking** on redb.  No background process is needed.

A future daemon mode can be layered on top for higher throughput, but the
daemonless mode must be fully correct and performant for typical agent
workloads (5-20 concurrent sessions).

### Architecture

```
┌────────────┐  ┌────────────┐  ┌────────────┐
│  Agent A    │  │  Agent B    │  │  Agent C    │
│  bash -c .. │  │  bash -c .. │  │  bash -c .. │
│             │  │             │  │             │
│ ┌────────┐  │  │ ┌────────┐  │  │ ┌────────┐  │
│ │ICloudFs│  │  │ │ICloudFs│  │  │ │ICloudFs│  │
│ │(in-proc)│  │  │ │(in-proc)│  │  │ │(in-proc)│  │
│ └───┬────┘  │  │ └───┬────┘  │  │ └───┬────┘  │
│     │       │  │     │       │  │     │       │
│ ┌───▼────┐  │  │ ┌───▼────┐  │  │ ┌───▼────┐  │
│ │ Engine │  │  │ │ Engine │  │  │ │ Engine │  │
│ │(memory)│  │  │ │(memory)│  │  │ │(memory)│  │
│ └───┬────┘  │  │ └───┬────┘  │  │ └───┬────┘  │
└─────┼───────┘  └─────┼───────┘  └─────┼───────┘
      │                │                │
      ▼                ▼                ▼
  ┌──────────────────────────────────────────┐
  │    redb (single file, file-locked)       │
  │    notes.redb / reminders.redb           │
  └──────────────────┬───────────────────────┘
                     │
      ┌──────────────┼──────────────┐
      ▼              ▼              ▼
  ┌────────┐    ┌────────┐    ┌────────┐
  │CloudKit│    │CloudKit│    │CloudKit│
  │(Notes) │    │(Remind)│    │(HME)   │
  └────────┘    └────────┘    └────────┘
```

Each process has its own in-memory engine.  They share state through **redb**
(on-disk) and **CloudKit** (remote).  No IPC.

### Phase Locking: Load → Operate → Save

Each bash session follows a three-phase protocol:

```
PHASE 1: LOAD (shared read lock on redb)
  ├─ fs2::lock_shared() on .redb.lock
  ├─ load_cache() from redb  (read transaction — multiple readers OK)
  ├─ fs2::unlock()
  ├─ if cache fresh (updated_at < max_age): skip sync
  └─ else: sync from CloudKit (no lock held — just HTTP calls)

PHASE 2: OPERATE (no redb lock held)
  ├─ Execute bash script against in-memory cache
  ├─ Reads: served from in-memory cache (zero I/O)
  ├─ Writes: call CloudKit API immediately (optimistic)
  │   ├─ On success: update in-memory cache
  │   └─ On conflict: reload record from CloudKit, retry (see below)
  └─ /tmp/ operations: purely in-memory, no external I/O

PHASE 3: SAVE (exclusive write lock on redb)
  ├─ if nothing changed: skip (no lock acquired)
  ├─ fs2::lock_exclusive() on .redb.lock
  ├─ Reload current redb state (merge point — see below)
  ├─ Apply our changes on top (incremental dirty tracking)
  ├─ save_cache() to redb (write transaction)
  └─ fs2::unlock()
```

**Key insight**: The redb lock is only held briefly during load and save, never
during the bash script execution or CloudKit API calls.  This means concurrent
sessions only block each other during the ~1ms redb I/O, not during the
~100-500ms CloudKit calls.

### Concurrent Read Path

Reads are served from the **in-memory cache** loaded in Phase 1.  No redb lock
is held during reads.  Multiple processes can read simultaneously because:

- `load_cache()` uses a **shared (read) lock** — multiple readers don't block
- After load, the lock is released immediately
- All reads during Phase 2 hit the in-memory HashMap (zero contention)

```
readdir(/Notes/)     → cache.folders.values()         — O(n), in-memory
stat(/Notes/x.md)    → cache.notes.get(id)            — O(1), in-memory
cat /Reminders/...   → cache.reminders.get(id)        — O(1), in-memory
cat /Notes/.../x.md  → engine.fetch_body(id)          — HTTP to CloudKit (no lock)
```

Note body fetches (`fetch_body`) go to CloudKit directly.  This is safe because
fetches are read-only and don't modify any shared state.

### Concurrent Write Path: Optimistic Concurrency

Writes go directly to CloudKit without holding any local lock.  CloudKit
provides built-in optimistic concurrency via `recordChangeTag`:

```
Process A                          CloudKit                         Process B
    │                                  │                                │
    ├─ load cache (tag="t1")           │                                │
    │                                  │      load cache (tag="t1") ────┤
    │                                  │                                │
    ├─ update note (tag="t1") ────────►│                                │
    │                                  │◄──── update note (tag="t1") ───┤
    │◄─── ok (new tag="t2") ──────────┤                                │
    │                                  ├──── CONFLICT (stale tag) ─────►│
    │                                  │                                │
    │                                  │      re-fetch record ──────────┤
    │                                  │◄──── (gets tag="t2")           │
    │                                  │                                │
    │                                  │◄──── retry update (tag="t2") ──┤
    │                                  ├──── ok (new tag="t3") ────────►│
```

**CloudKit is the source of truth for conflict resolution**, not our local
cache.  This means two processes can write to different records simultaneously
with zero coordination.  Conflicts only occur when two processes write to the
**same record** at the **same time** — which is rare for LLM agent workloads
(agents typically work on different notes).

### Conflict Detection and Retry

CloudKit modify operations include `recordChangeTag` as an optimistic lock.
When the server tag doesn't match, the operation fails.

```rust
async fn write_with_retry<F, Fut>(engine: &mut E, op: F, max_retries: u32) -> Result<()>
where
    F: Fn(&mut E) -> Fut,
    Fut: Future<Output = Result<()>>,
{
    for attempt in 0..=max_retries {
        match op(engine).await {
            Ok(()) => return Ok(()),
            Err(e) if is_conflict_error(&e) && attempt < max_retries => {
                // Re-sync just this record to get the latest change tag
                engine.sync(false).await?;
                continue;
            }
            Err(e) if is_record_not_found(&e) => {
                // Another process/device deleted the record
                // Remove from our cache and return ENOENT
                return Err(FsError::NotFound { .. });
            }
            Err(e) => return Err(e),
        }
    }
    Err(FsError::Other { message: "conflict persisted after retries".into() })
}
```

**Retryable errors:**
- `serverErrorCode` containing "CONFLICT" or change tag mismatch
- HTTP 409

**Non-retryable errors:**
- `RECORD_NOT_FOUND` → map to `ENOENT`
- HTTP 401/403 → map to `EIO: session expired`
- HTTP 429/503 → already handled by CloudKit client retry (existing logic)

### Save-Phase Merge (Handling Stale Sync Tokens)

When Phase 3 saves to redb, another process may have saved a newer sync token
in the meantime.  The merge protocol:

```
SAVE with merge:
  1. Acquire exclusive lock on .redb.lock
  2. Read current meta from redb:
     - stored_sync_token
     - stored_updated_at
  3. Compare with our sync_token:
     a. If stored_sync_token == our sync_token (or empty):
        → Normal save: write our full DirtyState to redb
     b. If stored_sync_token != our sync_token:
        → Another process synced while we were running.
        → We only write our item-level changes (dirty_items, dirty_names).
        → We do NOT overwrite sync_token or updated_at.
        → This preserves the other process's sync progress.
  4. Release lock
```

This is safe because:
- Item-level writes (dirty_items) are identified by CloudKit record ID
- Each process only dirties records it actually modified via CloudKit
- Two processes dirtying the same record can't happen without a CloudKit
  conflict (which is resolved in Phase 2, not Phase 3)

### `/tmp/` Isolation

Each bash session gets a private `/tmp/` backed by bashbox's `InMemoryFs`.
Sessions cannot see each other's temp files.

- `echo "draft" > /tmp/draft.md` in session A is invisible to session B
- Pipe workflows like `cat /Notes/Notes/x.md | jq . > /tmp/parsed.json` are
  session-local
- No coordination needed for `/tmp/` — it's purely per-process memory

### Rate Limiting

CloudKit enforces rate limits (HTTP 429 / 503 with `Retry-After` header).
The existing `CloudKitClient` already handles this with exponential backoff
and up to 6 retries.

For daemonless mode, each process handles its own rate limits independently.
Since CloudKit rate limits are per-account (not per-connection), concurrent
agents share the same budget.  In practice:

- **Reads** (sync, fetch_body) are lightweight and rarely rate-limited
- **Writes** are the bottleneck: ~10/second sustained per account
- If an agent gets 429'd, it backs off; other agents may still succeed
- No cross-process coordination needed — CloudKit is the rate limiter

**Default agent guidance** (enforced by execution limits, not locking):
- Max file size: 1 MB per write
- Max operations per session: 100,000 commands
- Agents should batch writes when possible (e.g., create multiple reminders
  in one script rather than spawning one process per reminder)

### Session Sharing

All processes share the same `SessionData` file on disk.  The session file is
read at process start (Phase 1) and never written by bash sessions — only
`icloud login` writes it.

If CloudKit returns 401/403:
1. The current operation returns `EIO: Session expired — run icloud login`
2. The process exits with code 3 (EXIT_AUTH)
3. Other concurrent processes may still be running with valid cached data
4. After `icloud login`, new processes pick up the refreshed session

### Startup Sequence

```
icloud bash -c "ls /Notes/"
  1. Load session from disk
  2. Load notes cache from redb (shared lock, ~1ms)
  3. Load reminders cache from redb (shared lock, ~1ms)
  4. If caches stale: sync from CloudKit (~200-500ms, no lock held)
  5. Create ICloudFs with caches + private InMemoryFs for /tmp/
  6. Run bash script against ICloudFs
  7. If caches dirty: save to redb (exclusive lock, ~1ms)
  8. Exit
```

Total overhead for a read-only command: ~5ms (cached) or ~500ms (first sync).
Concurrent processes only contend during steps 2-3 and 7, each ~1ms.

### Scalability Characteristics

| Metric | Daemonless | Notes |
|---|---|---|
| Concurrent readers | Unlimited | Shared redb read lock, ~1ms per load |
| Concurrent writers | 5-20 practical | Limited by CloudKit rate limits, not local locking |
| Lock contention | ~1ms per save | Exclusive lock only during redb write |
| CloudKit conflicts | Rare | Only when 2 agents edit the same record simultaneously |
| /tmp/ contention | Zero | Private per session |
| Memory per session | ~5-50MB | Cache + InMemoryFs for /tmp/ |

### Future: Optional Daemon Mode

For workloads exceeding 20 concurrent agents or requiring sub-millisecond
coordination, a daemon can be layered on top:

- Single process owns all engines behind `RwLock`
- Bash sessions connect via Unix socket
- Eliminates CloudKit conflicts entirely (single writer)
- Eliminates redb lock contention (single accessor)
- Adds background sync for external changes
- Trade-off: added complexity, deployment overhead

The daemon is **not needed for correctness** — only for performance at scale.
The daemonless protocol is the foundation that both modes share.
