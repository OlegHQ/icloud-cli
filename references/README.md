# References: iCloud private APIs and prior art

Curated index for implementing **icloud-cli** in Rust. Entries mirror the project spec (April 2026) plus additions for **Hide My Email** and **Notes** research.

| Language | Repository | Last known activity (approx.) | Use for this project |
|----------|------------|--------------------------------|----------------------|
| Go | [tarekbecker/icloud-reminders-cli](https://github.com/tarekbecker/icloud-reminders-cli) | 2026 (active) | **Primary** for Reminders CloudKit web flows and CLI behavior parity. |
| Node.js | [ElyaConrad/iCloud-API](https://github.com/ElyaConrad/iCloud-API) | 2023 (stale but complete) | Reminders + **Notes** HTTP shapes, session/2FA patterns. |
| Python | [picklepete/pyicloud](https://github.com/picklepete/pyicloud) | 2024 | Classic session + service modules; `services/reminders.py`. |
| Python | [mandarons/icloudpy](https://github.com/mandarons/icloudpy) | 2025 fork | Maintained fork of pyicloud; prefer for bugfixes. |
| Python | [namuan/pyremindkit](https://github.com/namuan/pyremindkit) | 2026 | Minimal Reminders-only wrapper. |
| Python | [glizzykingdreko/icloud-hme](https://github.com/glizzykingdreko/icloud-hme) | 2026 | **Hide My Email**: SRP auth, 2FA, rate limits. |
| Rust (this repo) | `crates/icloud-api` | — | Port/target implementation. |

## Topic files

- [reminders.md](reminders.md) — Reminders CRUD, lists, subtasks.
- [hide-my-email.md](hide-my-email.md) — SRP + alias lifecycle.
- [notes.md](notes.md) — Notes web API and on-disk/crypto research.

## Optional local clones

To study code without vendoring into git, use shallow clones (run from repo root, not committed):

```bash
mkdir -p references/vendor
git clone --depth 1 https://github.com/tarekbecker/icloud-reminders-cli.git references/vendor/icloud-reminders-cli
git clone --depth 1 https://github.com/ElyaConrad/iCloud-API.git references/vendor/iCloud-API
git clone --depth 1 https://github.com/mandarons/icloudpy.git references/vendor/icloudpy
git clone --depth 1 https://github.com/glizzykingdreko/icloud-hme.git references/vendor/icloud-hme
```

Add `references/vendor/` to `.gitignore` if you use this workflow.
