# icloud-cli

Rust CLI for iCloud Reminders, Notes, Hide My Email, and an automation-friendly virtual filesystem

`icloud-cli` is an unofficial command-line client for iCloud features that matter to scripts and agents. It uses the same private web and CloudKit-style endpoints as `icloud.com`; it does not use macOS EventKit or other Apple-only client frameworks.

## Status

This is a private-endpoint client. Apple can change the web APIs without notice, so release builds are best treated as automation tooling, not a compatibility promise.

Supported areas:

- Reminders: sync, list, list management, add, add-batch, edit, complete, and delete.
- Notes: sync, list, folders, get, create, update, move, delete, search, and Markdown export.
- Hide My Email: list, generate, reserve, deactivate, and delete aliases.
- Search: local full-text search across Notes, Reminders, and Hide My Email VFS paths.
- Bash/VFS: `icloud bash` exposes Notes, Reminders, Hide My Email, and `/tmp` through the embedded `bashbox` shell.

## Install

From a checkout:

```bash
cargo install --path crates/icloud-cli
```

For local development:

```bash
cargo build --workspace
cargo test --workspace
```

No `.envrc` or Homebrew-specific `LIBRARY_PATH` is required. The repo pins the stable Rust toolchain components in `rust-toolchain.toml`.

## Quick Start

Sign in and persist a reusable session:

```bash
icloud login --username you@example.com
```

For automation, avoid putting passwords in shell history or process lists:

```bash
pass show apple-id | icloud login --username you@example.com --password-stdin
```

Common commands:

```bash
icloud whoami
icloud reminders list today
icloud reminders add --list Inbox "Pay invoice" --due tomorrow
icloud notes list --folder Notes
icloud notes get "Project plan"
icloud hme list
icloud search "invoice" --path /Reminders
icloud bash -c 'find /Notes -name "*.md" | head'
```

Use `--json` for machine-readable output and `--plain` for stable tab-separated output. Diagnostics and hints go to stderr.

## Authentication And Storage

`icloud login` stores a reusable session under the platform config directory by default. Treat session files like credentials.

Secrets backends:

- `--secrets file` stores the full session in a local JSON file. This is the default and works in headless environments.
- `--secrets keychain` stores public session metadata on disk and secrets in the OS keychain.

2FA is explicit. Interactive use prompts for a trusted-device or SMS code when Apple requires it. Automation can pass `--code` or `ICLOUD_2FA_CODE`; use `--no-input` to fail instead of blocking.

## Virtual Filesystem

`icloud bash` runs `bashbox` against this layout:

```text
/
├── Notes/<Folder>/<Title>.md
├── Reminders/<List>/<Title>.md
├── HideMyEmail/aliases.json
└── tmp/
```

`/tmp` is in-memory per shell session. Notes and Reminders files are Markdown with YAML frontmatter where needed.

## Development

Install the pre-push hook:

```bash
./scripts/install-hooks.sh
```

On Windows PowerShell:

```powershell
./scripts/install-hooks.ps1
```

The hook runs:

```bash
cargo fmt --all
cargo clippy --workspace --all-targets --fix --allow-dirty --allow-staged --locked -- -D warnings
cargo test --workspace --locked
```

If formatting or clippy applies fixes, the hook stops the push so you can review and commit the generated changes. Set `ICLOUD_SKIP_PRE_PUSH=1` to bypass it intentionally.

Manual release-readiness checks:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
```

## Releases

Pushing a tag like `v0.1.0` runs `.github/workflows/release.yml` and publishes native archives for Linux, macOS Intel, macOS Apple Silicon, and Windows.

## Security Notes

- Do not commit Apple IDs, passwords, session files, cookies, CloudKit tokens, or keychain exports.
- This tool will not bypass Apple account security, captcha, rate limits, or abuse protections.
- Hide My Email generation appears rate-limited by Apple; expect roughly a handful of generated aliases per 30 minutes and an account-level alias ceiling.

## License

MIT. See [LICENSE](LICENSE).
