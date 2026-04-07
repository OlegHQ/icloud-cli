# Progress

Handoff log for cross-session work. New sessions should skim this before planning larger changes.

## Current architecture

- Workspace crates: `icloud-api`, `icloud-cli`, `icloud-bash`.
- `icloud bash` embeds **[bashbox](https://github.com/OlegHQ/bashbox)** (pinned git dependency in `Cargo.toml` files). A vendored `crates/just-bash/` tree is **not** part of this repository anymore.
- `icloud-bash::ICloudFs` implements `bashbox::fs::FileSystem`, composing Notes/Reminders sync engines, Hide My Email, and `bashbox::InMemoryFs` for `/tmp` and non-iCloud paths.

## What to update here

After substantial VFS, `icloud bash`, or bashbox integration work, record:

- what changed
- what was verified (commands run)
- what remains risky or incomplete
- sensible next tasks

## References

- Agent policy: `AGENTS.md`
- VFS semantics and roadmap detail: `crates/icloud-bash/SPEC.md`
