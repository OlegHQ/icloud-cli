# Progress

This file is the handoff record for the `icloud bash` / vendored `just-bash` cleanup.
Future sessions should read this first before rescanning the repo.

## Current State

- `icloud bash` wiring exists in `icloud-cli` and `icloud-bash`.
- The vendored `crates/just-bash/` fork is being cleaned up with the goal of keeping it standalone and releasable later.
- `just-bash` must stay generic. iCloud-specific behavior belongs only in `icloud-bash` and `icloud-cli`.
- The vendored `jq` / `yq` implementations now route through `jaq` instead of the deleted homegrown query engine.

## Completed In This Session

### Boundary / policy

- Added an explicit module-boundary rule to `AGENTS.md`:
  `just-bash` must not depend on `icloud-*` crates or encode iCloud semantics.

### Vendored `just-bash` cleanup

- Added feature gates in `crates/just-bash/Cargo.toml`:
  - `cli`
  - `sandbox`
- Made the standalone binary require the `cli` feature.
- Gated the sandbox module/re-export in `crates/just-bash/src/lib.rs`.
- Removed the dead `commands/registry.rs` module and its re-exports from `commands/mod.rs`.
  That file was 492 lines of cumulative batch-registration boilerplate and appeared unused.
- Replaced hand-rolled base64 and hex encode/decode logic in `crates/just-bash/src/fs/types.rs`
  with the `base64` and `hex` crates already appropriate for the job.

### `icloud-bash` VFS fixes

- Implemented a shared, deterministic filename disambiguation path in `crates/icloud-bash/src/vfs.rs`.
- Fixed a correctness bug where duplicate note/reminder titles could resolve to the wrong record ID.
- Reused the same disambiguation helper for:
  - path lookup
  - directory listing
  - glob/path enumeration
- Kept the VFS logic in `icloud-bash`; no just-bash coupling was introduced.

### Test runner prep

- Added `.config/nextest.toml` so the repo is ready to use Cargo Nextest.
- Installed `cargo-nextest 0.9.132` via Homebrew and exercised the vendored crate under
  nextest isolation.
- `cargo install cargo-nextest --locked` failed in this environment because `aws-lc-sys`
  could not find `CoreServices/CoreServices.h` in the active Xcode SDK, so Homebrew was
  the pragmatic install path.

### jq / yq replacement

- Replaced the vendored `jq` command with a thin `jaq`-backed implementation in
  `crates/just-bash/src/commands/jq/mod.rs`.
- Replaced the vendored `yq` command with an `lq`-style wrapper around `jaq`, using YAML as the
  default input format, jq/JSON as the default stdout format, and preserving source format for
  `--inplace`.
- Added `crates/just-bash/src/commands/jaq_support.rs` as the shared compile / parse / format
  helper for both commands.
- Deleted the old custom query stack under `crates/just-bash/src/commands/query_engine/`.
- Deleted the old hand-rolled `crates/just-bash/src/commands/yq/formats.rs` format conversion
  layer and dropped the direct `serde_yaml`, `toml`, and `indexmap` dependency usage from
  `crates/just-bash/Cargo.toml`.
- Updated the vendored `CLAUDE.md` note so it no longer claims `query_engine/` exists.

## Verification Run

Passed:

- `cargo fmt --all`
- `cargo test -p icloud-bash`
- `cargo test -p just-bash --lib fs::types::tests::`
- `cargo test -p just-bash --lib bash::tests::test_bash_new_default`
- `cargo test -p just-bash --features cli,sandbox --no-run`
- `cargo check -p just-bash`
- `cargo fmt --package just-bash`
- `cargo nextest run -p just-bash --lib -E 'test(/^commands::jq::tests::/)'`
- `cargo nextest run -p just-bash --lib -E 'test(/^commands::yq::tests::/)'`
- `cargo nextest run -p just-bash --lib`
- `cargo test -p just-bash --lib`

Current status:

- The old monolithic `cargo test -p just-bash --lib` abort is gone.
- `cargo nextest run -p just-bash --lib` now passes cleanly:
  - 2691 passed
  - 0 failed
  - 0 skipped

## Findings Report

### Fixed

- `crates/just-bash/src/commands/registry.rs`
  - Category: duplication / boilerplate
  - Severity: high
  - Issue: cumulative `register_batch_*` / `create_batch_*` scaffolding with no observed external use
  - Fix: removed the module and its re-exports

- `crates/just-bash/src/fs/types.rs`
  - Category: hand-rolled
  - Severity: high
  - Issue: local reimplementation of base64 and hex encoding/decoding
  - Fix: replaced with `base64` and `hex`

- `crates/icloud-bash/src/vfs.rs`
  - Category: duplication / correctness / perf
  - Severity: high
  - Issue: multiple independent title→filename paths with inconsistent collision handling
  - Fix: extracted one deterministic disambiguation path and reused it

- `crates/just-bash/src/commands/query_engine/`
  - Category: hand-rolled / over-budget
  - Severity: critical
  - Issue: custom jq/yq parser and evaluator stack that was both large and the source of the
    isolated `yq` failure cluster under nextest
  - Fix: replaced the stack with `jaq`, deleted the old modules, and revalidated the full crate

- `crates/just-bash/src/commands/jq/mod.rs` and `crates/just-bash/src/commands/yq/mod.rs`
  - Category: over-budget / duplicated behavior
  - Severity: high
  - Issue: large command-specific implementations duplicated query execution and format handling
  - Fix: collapsed both commands onto a shared `jaq_support` helper and reduced them to thin CLI
    wrappers

### Still Open

- `crates/just-bash/src/parser/lexer.rs`
  - Category: over-budget
  - Severity: critical
  - Issue: ~2380 LOC
  - Fix: split token scanning by lexical domain

- `crates/just-bash/src/parser/parser.rs`
  - Category: over-budget
  - Severity: critical
  - Issue: ~2312 LOC
  - Fix: split parser entry orchestration from grammar-specific parsing

- `crates/just-bash/src/commands/find/mod.rs`
  - Category: over-budget
  - Severity: high
  - Issue: ~2097 LOC
  - Fix: move traversal, predicate evaluation, and action execution into separate files

- `crates/just-bash/src/commands/awk/parser.rs`
  - Category: over-budget
  - Severity: high
  - Issue: ~2035 LOC
  - Fix: split expressions, statements, and function parsing

- `crates/just-bash/src/commands/tar/mod.rs`
  - Category: over-budget
  - Severity: high
  - Issue: ~1958 LOC
  - Fix: move create/list/extract/update flows behind separate modules or strategy types

- `crates/just-bash/src/commands/curl/mod.rs`
  - Category: over-budget
  - Severity: high
  - Issue: ~1845 LOC
  - Fix: split request building, response formatting, and option parsing

- `crates/just-bash/src/commands/gzip/mod.rs`
  - Category: over-budget
  - Severity: high
  - Issue: ~1634 LOC
  - Fix: split shared archive helpers from gzip/gunzip/zcat entrypoints

- `crates/just-bash/src/fs/in_memory_fs.rs`
  - Category: over-budget
  - Severity: high
  - Issue: ~1601 LOC
  - Fix: split path normalization, metadata ops, and mutating FS ops

- `crates/just-bash/src/interpreter/types.rs` and `crates/just-bash/src/interpreter/interpreter.rs`
  - Category: duplication / dead-code risk
  - Severity: high
  - Issue: overlapping interpreter abstractions exist in parallel, including duplicated `InterpreterContext`
  - Fix: collapse to a single owning module and remove shadow abstractions

## Architecture Decision For Next Work

Use a domain-component structure for the cleanup, not a giant top-level layered rewrite.

- `just-bash` domains already exist naturally:
  - `parser`
  - `interpreter`
  - `commands`
  - `fs`
  - `sandbox`
  - `network`
- The next refactors should split god modules inside those domains.
- Do not move `just-bash` toward `icloud-*` abstractions.

## Next Session Plan

### Phase 1: Remove remaining duplicated interpreter abstractions

1. Audit `crates/just-bash/src/interpreter/types.rs`
2. Audit `crates/just-bash/src/interpreter/interpreter.rs`
3. Merge duplicated context/command abstractions into one owning module.
4. Delete dead aliases and shadow types once callers are updated.

### Phase 2: Tackle one god module at a time

Recommended order:

1. `commands/find/mod.rs`
2. `commands/gzip/mod.rs`
3. `commands/curl/mod.rs`
4. `fs/in_memory_fs.rs`
5. `parser/lexer.rs`

Rule:

- one file family per session
- keep changes mechanical and verifiable
- avoid broad rewrites across parser + interpreter + commands in the same pass

### Phase 3: Start reducing blanket suppression

1. Keep the current blanket `#![allow(...)]` for now.
2. After the structural cleanup starts landing, chip away module-by-module.
3. Do not try to remove all lint suppressions in one session.

## Handoff Notes

- `git status` currently shows the vendored `just-bash` files as untracked in this worktree.
  Be careful reviewing diffs because `git diff --stat` will underreport untracked files.
- The VFS file is large because this branch already had a substantial in-progress implementation.
  Do not treat all of `crates/icloud-bash/src/vfs.rs` as new work from this session.
- If starting a fresh session, begin with:
  1. `cat PROGRESS.md`
  2. `git status --short`
  3. `cargo nextest --version` or install it if missing
