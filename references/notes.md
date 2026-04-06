# Notes (iCloud web)

**Notes** are higher risk than Reminders: rich text, attachments, optional encryption, and collaboration complicate a first Rust implementation.

## API / transport references

- **[ElyaConrad/iCloud-API](https://github.com/ElyaConrad/iCloud-API)** — Includes a **Notes** section in the README and demo code. Use for initial **list/read/create/update** JSON shapes over the same iCloud session as other services. Stale but structurally useful.

- **[mandarons/icloudpy](https://github.com/mandarons/icloudpy)** / **[picklepete/pyicloud](https://github.com/picklepete/pyicloud)** — Often expose a Notes-related service module; cross-check cookies and hostnames.

## Schema / crypto (often backup-oriented)

- **[threeplanetssoftware/apple_cloud_notes_parser](https://github.com/threeplanetssoftware/apple_cloud_notes_parser)** — Deep understanding of Notes **record types**, encryption, and attachments as stored in iCloud/CloudKit-shaped data. Valuable when moving beyond plaintext stubs.

## Staged “full featured” definition

1. **Baseline:** folders/lists, get note metadata, read body as plain or minimal HTML.  
2. **CRUD:** create note, update title/body, delete/move.  
3. **Advanced:** attachments, encrypted notes, shared notes (explicit non-goals until baseline is solid).
