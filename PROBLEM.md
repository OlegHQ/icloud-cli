# Apple Notes TopoText Update Protocol

## Problem

`icloud notes update`, `icloud cp` to `icloud:/Notes/...`, and VFS note writes
currently rewrite `TextDataEncrypted` from Markdown. Native Apple Notes has
shown a duplicate-body artifact after these edits: the visible note body appears
again at the bottom after sync.

`TextDataEncrypted` is not just compressed visible text. iCloud web names it
`EncryptedTopoTextEncoding`, and HAR captures show browser edits preserve and
extend a TopoText/CRDT archive. A full fix needs enough fixtures to implement
browser-style edit operations instead of whole-body replacement.

## Capture Rules

- Use disposable notes only. Do not capture private notes.
- Use Chrome/Chromium DevTools Network with "Preserve log" enabled.
- Disable cache in DevTools while capturing.
- Capture one scenario per HAR file. Name files with the scenario ID below.
- Start each scenario from a fresh browser tab at `https://www.icloud.com/notes`.
- Wait until the note list and selected note finish loading before editing.
- After each edit, click away to another note or wait until the network quiets
  for at least 5 seconds.
- Export the full HAR with content.
- Do not share raw HARs publicly; they contain cookies/tokens and note content.
- For each HAR, also save a small text sidecar with:
  - scenario ID
  - exact initial note body
  - exact final note body
  - whether native Notes was open during capture
  - whether iCloud web was refreshed before editing

## Baseline Browser Flow

Use this exact flow for browser-only scenarios unless the scenario says
otherwise:

1. Open a new browser tab.
2. Open `https://www.icloud.com/notes`.
3. Open DevTools Network.
4. Enable "Preserve log" and "Disable cache".
5. Clear the network log.
6. Create or open the disposable note named by the scenario.
7. Wait until the editor is fully loaded and no Notes requests are active.
8. Clear the network log again.
9. Perform the exact edit.
10. Click another note, then click back to the edited note.
11. Wait for network quiet.
12. Export HAR with content.
13. Record the sidecar notes.

## Required HAR Scenarios

### N00 Create Plain Note

Initial state: no note exists.

Steps:
1. Create a new note in iCloud web.
2. Type exactly:

```text
TopoText N00

alpha
beta
gamma
```

Expected purpose: capture browser create payload and initial archive shape.

### N01 Append Text At End

Initial note:

```text
TopoText N01

alpha
beta
gamma
```

Edit:
1. Place cursor after `gamma`.
2. Press Enter.
3. Type `delta`.

Expected final:

```text
TopoText N01

alpha
beta
gamma
delta
```

Purpose: identify append operation shape, clock update, and replica metadata.

### N02 Insert Text In Middle

Initial note:

```text
TopoText N02

alpha
gamma
```

Edit:
1. Place cursor at the beginning of the `gamma` line.
2. Type `beta` and press Enter.

Expected final:

```text
TopoText N02

alpha
beta
gamma
```

Purpose: identify insertion before existing text.

### N03 Replace Word In Middle

Initial note:

```text
TopoText N03

alpha beta gamma
```

Edit:
1. Select only `beta`.
2. Type `BETA-EDITED`.

Expected final:

```text
TopoText N03

alpha BETA-EDITED gamma
```

Purpose: identify replace as delete+insert or another operation shape.

### N04 Delete Line

Initial note:

```text
TopoText N04

alpha
delete-me
gamma
```

Edit:
1. Select the entire `delete-me` line including its newline.
2. Press Backspace.

Expected final:

```text
TopoText N04

alpha
gamma
```

Purpose: identify delete operation shape and boundary behavior.

### N05 Edit Title

Initial note:

```text
TopoText N05

body
```

Edit:
1. Change only the title line to `TopoText N05 Edited`.

Expected final:

```text
TopoText N05 Edited

body
```

Purpose: identify title/body field coupling between `TitleEncrypted`,
`SnippetEncrypted`, and `TextDataEncrypted`.

### N06 Checklist Toggle

Initial note:

```text
TopoText N06

☐ alpha
☐ beta
☑ gamma
```

Creation detail:
Create these as real Apple Notes checklist items with the toolbar, not as typed
Unicode checkbox characters.

Edit:
1. Toggle only `beta` from incomplete to complete.

Expected purpose: identify checklist metadata update without visible text
change.

### N07 Checklist Insert Item

Initial note:

```text
TopoText N07

☐ alpha
☐ gamma
```

Creation detail:
Use real Apple Notes checklist items.

Edit:
1. Place cursor after `alpha`.
2. Press Enter.
3. Type `beta`.

Expected purpose: identify checklist insertion and generated checklist item IDs.

### N08 Bullet List Insert

Initial note:

```text
TopoText N08

- alpha
- gamma
```

Creation detail:
Use real Apple Notes bullet list formatting.

Edit:
1. Insert a new bullet `beta` between `alpha` and `gamma`.

Purpose: distinguish list paragraph operation shape from plain text.

### N09 Rich Formatting Change

Initial note:

```text
TopoText N09

alpha beta gamma
```

Edit:
1. Select only `beta`.
2. Apply bold.

Purpose: identify attribute-run-only update without text change.

### N10 Link Add

Initial note:

```text
TopoText N10

Open example
```

Edit:
1. Select `example`.
2. Add link `https://example.com/`.

Purpose: identify link attribute encoding and update operation shape.

### N11 Table Cell Edit

Initial note:

```text
TopoText N11
```

Creation detail:
Use Apple Notes table insertion to create a 2x2 table:

```text
A1 | B1
A2 | B2
```

Edit:
1. Change cell `B2` to `B2 edited`.

Purpose: determine whether table edits touch Note `TextDataEncrypted`,
Attachment `MergeableDataEncrypted`, or both.

### N12 Repeated Edits Same Session

Initial note:

```text
TopoText N12

one
```

Edits without refreshing:
1. Append line `two`.
2. Wait for save/network quiet.
3. Append line `three`.
4. Wait for save/network quiet.

Purpose: identify how replica clocks advance across multiple web edits from
one loaded editor session.

### N13 Refresh Between Edits

Initial note:

```text
TopoText N13

one
```

Edits:
1. Append line `two`.
2. Wait for save/network quiet.
3. Refresh the browser tab.
4. Reopen the same note.
5. Append line `three`.

Purpose: compare replica reuse after browser reload.

### N14 Native Interop After Browser Edit

Initial note:

```text
TopoText N14

browser line
```

Flow:
1. Keep native Apple Notes closed.
2. In iCloud web, append `web edit`.
3. Wait for sync.
4. Open native Apple Notes.
5. Confirm visible final body in the sidecar.

Purpose: establish known-good browser-to-native rendering baseline.

### N15 Native Edit Then Browser Save

Initial note:

```text
TopoText N15

native baseline
```

Flow:
1. Open native Apple Notes.
2. Append `native edit`.
3. Wait until iCloud web sees the native edit.
4. Start HAR capture in browser.
5. In iCloud web, append `web edit`.
6. Export HAR.

Purpose: capture browser save after native-created TopoText state.

## Optional CLI Reproduction Capture

These are not browser reverse-engineering fixtures, but they help compare our
payloads against web payloads.

### C01 CLI Update Then Browser Observe

1. Create a disposable note in iCloud web with checklist-heavy content.
2. Record the note ID.
3. Run `target/debug/icloud --no-input --max-age 0 notes update <id>` with a
   small body edit.
4. Force refresh iCloud web and native Apple Notes.
5. Record whether duplication appears.

### C02 CLI Update HAR Through Proxy

If using an HTTP proxy capable of capturing the CLI request:

1. Capture only `database/1/com.apple.notes/production/private/records/modify`.
2. Redact cookies/tokens.
3. Compare operation field names and `TextDataEncrypted` structure with N01-N15.

## What To Extract From Each HAR

For each scenario, extract these redacted facts:

- `records/modify` request and response for `recordType: "Note"`.
- Note record field names and wrapper shapes.
- `TextDataEncrypted` before and after, decoded to:
  - compression type
  - visible text
  - attribute run count and paragraph styles
  - TopoText operation records
  - replica/vector metadata entries
- Any `Attachment` records modified in the same scenario.
- Whether `TitleEncrypted`, `SnippetEncrypted`, `CreationDate`,
  `ModificationDate`, `FoldersModificationDate`, `shortGUID`, `TextDataAsset`,
  or attachment fields changed.

## Implementation Acceptance Criteria

- Existing-note updates produce browser-equivalent TopoText operation deltas for
  append, insert, replace, delete, checklist toggle, list insert, and rich
  formatting changes.
- Native Apple Notes does not duplicate content after CLI/VFS edits.
- Browser edits after CLI edits do not duplicate or lose content.
- CLI rejects unsupported rich/attachment states with a clear error instead of
  corrupting or flattening them.
- Regression fixtures are redacted and committed; raw HARs remain local only.
