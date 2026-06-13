---
name: icloud-cli-workflows
description: "Recipe book for the installed icloud CLI's non-obvious automation workflows: session/cache discipline, bashbox VFS path semantics, icloud cp import/export shapes, Notes Markdown/frontmatter roundtrips, Reminders smart filters/frontmatter edits, local search indexing, and Hide My Email alias lifecycle. Use when Codex needs to operate iCloud Notes, Reminders, Hide My Email, search, or the iCloud virtual filesystem with multi-step workflows rather than only look up command syntax."
---

# iCloud CLI Workflows

Use this skill to compose reliable `icloud` CLI workflows from behavior that is easy to miss in `--help`. Baseline analyzed: installed `icloud 0.1.2`; re-check `icloud version` when exact behavior matters.

## First Moves

- Prefer `icloud --no-input --quiet whoami` before automation. Do not run `login` unless the user explicitly asks or provides credentials.
- Put diagnostics on stderr out of your parsing path: command results are stdout; hints, prompts, progress, dry-run previews, and JSON error reports are stderr.
- Use `--json` when selecting IDs or fields programmatically. Use `--plain` only for stable tab-separated rows without headers. Use `--quiet` for counts/status only.
- Use `--max-age 0` when freshness matters. Raise `--max-age` only for read-heavy workflows where stale cache is acceptable.
- Treat `--session`, redb DB files, cookies, tokens, and keychain-backed session metadata as credentials. Never print or commit them.
- Prefer one `icloud bash -c '...'` or one `icloud cp -r ...` over many short processes when doing related writes; each process loads/syncs/saves caches.

## Session And Cache Recipes

Use a forced sync when the cache appears inconsistent:

```bash
icloud --no-input --max-age 0 notes sync --force
icloud --no-input --max-age 0 reminders sync --force
```

Use `--secrets keychain` only when the host has working keychain access. Use default `--secrets file` for headless/CI contexts. For login automation, prefer:

```bash
pass show apple-id | icloud login --username "$APPLE_ID" --password-stdin --no-input --code "$ICLOUD_2FA_CODE"
```

Avoid `--password` except in throwaway manual contexts; it can expose the password through process listings.

## VFS Rules That Change Workflows

The VFS mounted by `icloud bash` and used by `icloud cp` is:

```text
/
  Notes/<Folder>/<Title>.md
  Reminders/<List>/<Title>.md
  HideMyEmail/aliases.json
  tmp/
```

Apply these rules:

- Quote iCloud paths; folder/list/note titles commonly contain spaces.
- Notes and Reminders files must be `.md` and valid UTF-8.
- `/tmp` is per `icloud bash` session, in-memory, and disappears after the session.
- `/HideMyEmail/aliases.json` is read-only. Mutate aliases with `icloud hme ...`.
- Cross-service moves are invalid. Copy/export through host files or `/tmp` and create the destination service record.
- VFS file resolution is scoped to the requested folder/list. Same-title Notes or Reminders in another folder/list are not fallback targets.
- Filename title mapping is not byte-for-byte: `/` in titles becomes a division slash, empty titles become `Untitled`, long names are truncated, and collisions get ` (2)`, ` (3)`, etc.

## Host To iCloud Transfer Recipes

Use `icloud cp` when the task is import/export, not shell exploration. It requires exactly one `icloud:/...` endpoint.

Export one file:

```bash
icloud cp 'icloud:/Notes/Work/Plan.md' ./Plan.md
icloud cp 'icloud:/HideMyEmail/aliases.json' ./aliases.json
```

Export a whole service or subtree:

```bash
icloud cp -r 'icloud:/Notes' ./icloud-notes
icloud cp -r 'icloud:/Reminders/Home' ./home-reminders
```

Import one Markdown file, creating the destination folder/list if needed:

```bash
icloud cp ./Plan.md 'icloud:/Notes/Work/Plan.md'
icloud cp ./Task.md 'icloud:/Reminders/Home/Task.md'
```

Import a directory into a service root only when the local tree shape is exactly one collection level deep:

```text
local-notes/
  Work/*.md
  Personal/*.md
```

```bash
icloud cp -r ./local-notes 'icloud:/Notes'
```

Do not import nested folders under a Notes folder or Reminders list; the CLI rejects them because those services expose only `Folder/File.md` or `List/File.md`.

## Notes Recipes

Create or update multiline Notes through stdin or `icloud cp`; avoid `--body` for multiline shell text unless quoting is deliberate:

```bash
printf '# Trip Plan\n\n- flights\n- hotel\n' | icloud notes create --folder Travel
printf '# Trip Plan\n\nUpdated body\n' | icloud notes update '<note-id-or-unique-prefix>'
```

For local editing, round-trip through the VFS:

```bash
icloud cp 'icloud:/Notes/Travel/Trip Plan.md' ./trip.md
$EDITOR ./trip.md
icloud cp ./trip.md 'icloud:/Notes/Travel/Trip Plan.md'
```

Remember these Notes-specific behaviors:

- The first Markdown heading controls the Apple Notes title. Changing `# Title` can change the VFS filename after sync.
- VFS Note frontmatter (`id`, `folder`, `modified`) is regenerated and effectively read-only. Move folders with `icloud notes move` or VFS `mv`, not by editing frontmatter.
- Supported rich Markdown round-trips include headings, bold, italic, bold+italic, strikethrough, underline, links, bullet lists, checklists, and pipe tables.
- Attachments, collaboration metadata, and password-protected notes are outside the reliable writable surface.
- `icloud notes delete` and folder deletion move Notes to Apple Notes Recently Deleted. The active CLI/VFS/search/export surfaces hide them, but there is no hard purge command.
- `icloud notes search` searches cached title/snippet metadata. Use top-level `icloud search` with `--rebuild` for local full-text search of VFS paths and contents.
- Prefer IDs from `icloud notes list --json` for destructive or ambiguous updates; title prefixes can collide.

## Reminders Recipes

For simple batch creation, prefer the API batch command:

```bash
icloud reminders add-batch --list Inbox 'pay rent' 'renew tags' 'email Sam'
```

Use VFS Markdown files for bulk edits that combine title, notes, due date, priority, and completion:

```markdown
---
completed: false
due: "2026-06-20"
priority: "high"
notes: "Bring receipt"
---
Return router
```

```bash
icloud cp ./return-router.md 'icloud:/Reminders/Errands/Return router.md'
```

Apply these Reminders-specific rules:

- The Markdown body is the reminder title. Editing body text on an existing VFS file renames the reminder.
- Writable frontmatter is `completed`, `due`, `priority`, and `notes`. `id` and `list` are not edited through frontmatter.
- Set `due: null` in VFS frontmatter, or use `icloud reminders edit <id> --clear-due`, to clear a due date.
- Move between lists with VFS `mv` or recreate through CLI; editing `list:` in frontmatter is ignored.
- `reminders list today` means incomplete reminders due up to today, so it includes overdue items. `tomorrow` is only tomorrow, `week` is today through seven days out, `upcoming` means incomplete reminders that have any due date, `completed` means completed only, and `all` includes completed.
- `complete --dry-run` and `delete --dry-run` print previews to stderr, not stdout. Capture both streams if the preview matters.
- Use parent IDs or unique prefixes for subtasks with `--parent`; get stable IDs from `icloud reminders list --json --all`.
- List deletion deletes child reminders before parents, then removes the list.

## Search Recipes

Use top-level `icloud search` for full-text search across VFS paths and contents:

```bash
icloud search 'router' --path 'icloud:/Reminders/Errands' --rebuild --json
icloud search 'example.com' --service hme --json
```

Search behavior to remember:

- `--path` accepts VFS scopes such as `/Notes/Work`, `icloud:/Reminders/Home`, or `/HideMyEmail`.
- `--service` and `--path` must agree; `--service notes --path /Reminders` is an error.
- Use `--rebuild` after recent Notes/Reminders/HME changes if the index may be stale.
- HME has no reusable local cache; indexing HME refetches aliases and skips index writes only when the alias fingerprint is unchanged.
- Use `--index <dir>` to isolate experiments from the default search index.

## Hide My Email Recipes

Generation is a two-step lifecycle:

```bash
email="$(icloud hme generate --plain | head -n1)"
icloud hme reserve "$email" --label 'Vendor name' --note 'Created by automation'
```

Operational rules:

- `generate` only returns an address candidate. It is not active until `reserve`.
- If `reserve --label` is omitted, the CLI uses the local part before `@`.
- Use `icloud hme list --json` or `--plain` to get `anonymous_id`; deactivate/delete operate on that ID, not the email address.
- `deactivate` is reversible from Apple's service surface; `delete` is permanent.
- Expect tight Apple-side generation limits, roughly a handful per 30 minutes and an account-level alias ceiling. Do not bulk generate speculative aliases.
- For read-only audits, `icloud cp 'icloud:/HideMyEmail/aliases.json' ./aliases.json` avoids learning the HME API shape.

## Bash VFS Recipes

Use `icloud bash` when a task benefits from one mounted snapshot and write-through session:

```bash
icloud bash -c 'ls /; ls "/Notes/Work"; cat "/Reminders/Inbox/Pay rent.md"'
```

Use stdin for longer scripts:

```bash
icloud bash - <<'SH'
cat "/Notes/Work/Plan.md" > /tmp/plan.md
cat /tmp/plan.md
SH
```

Know the shell constraints:

- It is bashbox, not the host shell. Do not rely on arbitrary external binaries; use the supported shell builtins and VFS commands available in bashbox.
- Non-interactive execution prints the bash result stdout/stderr, then exits with the script exit code.
- Interactive mode has an `edit <path>` builtin that opens a VFS file through the configured editor, then writes it back.
- The environment starts with `HOME=/`, `USER=icloud`, `ICLOUD_SESSION`, `ICLOUD_NOTES_COUNT`, and `ICLOUD_REMINDERS_COUNT`.
- There is no background sync during a shell session. Reads use the initial cache snapshot, except Notes body fetches; writes call CloudKit immediately and the updated cache is saved when the session exits.
- Keep each iCloud file write under 1 MB.

## Failure Recovery

- Session/auth errors exit 3. Transport, upstream, or cache errors exit 4. Usage/domain errors usually exit 2.
- On session expiry, run an explicit `icloud login`; do not loop retries that trigger login storms.
- On stale search results, rebuild the index. On stale object metadata, force-sync the relevant service. On ambiguity, switch from title prefixes to IDs.
- On CloudKit conflict or rate pressure, retry at workflow boundaries after a short delay; avoid parallel writes to the same Note or Reminder.
