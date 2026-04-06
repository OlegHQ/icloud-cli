# Reminders (iCloud web / CloudKit)

## Primary reference

- **[tarekbecker/icloud-reminders-cli](https://github.com/tarekbecker/icloud-reminders-cli)** (Go)  
  - Full **CRUD**, lists/collections, hierarchical **subtasks**, batch create.  
  - Native **CloudKit web** implementation; **2FA** and **session caching**.  
  - Pre-built binaries / Homebrew: see project README.  
  - **Rust port goal:** Match observable behavior and HTTP payloads where practical.

## Secondary references

- **[ElyaConrad/iCloud-API](https://github.com/ElyaConrad/iCloud-API)** — JS `Reminders` namespace: `getOpenTasks`, `getCompletedTasks`, `createTask`, `changeTask`, `deleteTask`, `completeTask`, collection create/change/delete. Useful for JSON field names if Go source differs.

- **[mandarons/icloudpy](https://github.com/mandarons/icloudpy)** / **[picklepete/pyicloud](https://github.com/picklepete/pyicloud)** — `services/reminders.py`: classic webservice-style access; good for cookie/session expectations.

- **[namuan/pyremindkit](https://github.com/namuan/pyremindkit)** — Small surface area; quick cross-check for create/update fields (title, due date, notes, priority, URL, list ID).

## Rust implementation notes

- Implement Reminders in `icloud-api` as a dedicated module using **async** `reqwest` with the same **session** jar as the rest of the crate.
- Prefer **integration tests** behind `ICLOUD_CLI_INTEGRATION=1` with real credentials omitted from CI.
