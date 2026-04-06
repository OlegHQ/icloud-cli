# CRATE_SWAP_PLAN

This document is the implementation plan for the final simplification pass.
It is written for a parallel squad of 10+ Claude-style coding agents working on the same codebase in separate branches.

The target is `crates/just-bash/`.
That crate dominates the remaining code-size debt and contains the largest concentration of hand-rolled implementations.

## Goal

- Keep all current features and test-covered behavior.
- Reduce code size aggressively by replacing hand-rolled implementations with crates or shared utilities.
- Simplify large modules enough that future cleanup is mostly modularization, not archaeology.
- Preserve the current architectural boundary:
  `just-bash` stays generic and must not depend on `icloud-*` crates or encode iCloud-specific behavior.

## Non-Goals

- Replacing the bash parser or interpreter with an external shell library.
- Replacing the awk or sed implementations wholesale.
- Changing CLI behavior unless the existing tests already permit it.
- Broad formatting-only churn across the workspace.
- Touching `icloud-api`, `icloud-cli`, or `icloud-bash` unless required for compilation.

## Baseline

As of this plan:

- `cargo nextest run -p just-bash --lib` passes cleanly.
- `cargo test -p just-bash --lib` passes cleanly.
- `crates/just-bash` is the main simplification target.

Workspace hotspots by file size:

| File | LOC | Notes |
|---|---:|---|
| `crates/just-bash/src/parser/lexer.rs` | 2380 | bash lexer, not a crate-swap target |
| `crates/just-bash/src/parser/parser.rs` | 2312 | bash parser driver, not a crate-swap target |
| `crates/just-bash/src/commands/find/mod.rs` | 2097 | traversal, actions, output mixed together |
| `crates/just-bash/src/commands/awk/parser.rs` | 2035 | custom language parser, not a crate-swap target |
| `crates/just-bash/src/commands/tar/mod.rs` | 1958 | candidate for partial crate swap |
| `crates/just-bash/src/commands/curl/mod.rs` | 1845 | candidate for typed URL/HTTP cleanup |
| `crates/just-bash/src/commands/gzip/mod.rs` | 1634 | uses `flate2`, but still hand-rolls too much |
| `crates/just-bash/src/fs/in_memory_fs.rs` | 1601 | internal refactor target, not a crate-swap target |
| `crates/just-bash/src/commands/md5sum/mod.rs` | 805 | obvious crate swap |
| `crates/just-bash/src/commands/html_to_markdown_cmd.rs` | ~250 | obvious crate swap |

## High-Confidence Crate Swaps

These are the swaps with the best deletion-to-risk ratio.

### 1. Checksum commands

Current:

- `crates/just-bash/src/commands/md5sum/mod.rs`
- Contains full hand-rolled implementations of MD5, SHA-1, and SHA-256.

Replace with:

- `md-5`
- `sha-1`
- `sha2`

Why:

- These crates are pure Rust and expose the same `Digest`-style API.
- This should delete the entire handwritten hash core while preserving output format.

References:

- `md-5`: https://docs.rs/md-5/latest/md5/
- `sha-1`: https://docs.rs/sha-1/latest/sha1/
- `sha2`: https://docs.rs/sha2/latest/sha2/

Expected line reduction:

- Roughly 400-650 LOC

### 2. Tar archive read/write

Current:

- `crates/just-bash/src/commands/tar/archive.rs`
- `crates/just-bash/src/commands/tar/mod.rs`
- Custom tar header serialization, parsing, checksums, path splitting, and archive walking logic.

Replace core archive format handling with:

- `tar`

Keep custom:

- Virtual filesystem traversal
- CLI option semantics
- Output formatting
- Deterministic entry ordering

Why:

- The `tar` crate already provides `Archive`, `Builder`, and `Header`.
- We should stop maintaining tar header encoding and checksum logic ourselves.

Reference:

- `tar`: https://docs.rs/tar/latest/tar/

Expected line reduction:

- Roughly 500-900 LOC combined across `archive.rs` and `mod.rs`

### 3. Curl URL parsing and URL encoding

Current:

- `crates/just-bash/src/commands/curl/mod.rs`
- `crates/just-bash/src/commands/curl/parse.rs`
- `crates/just-bash/src/commands/curl/form.rs`
- Manual URL normalization and percent encoding.

Replace with:

- `url`

Use for:

- `Url::parse`
- `Url::join`
- `url::form_urlencoded`
- remote-name extraction from parsed path segments

Why:

- This eliminates hand-rolled URL normalization and encoding.
- It also reduces edge-case behavior around relative redirects and path extraction.

Reference:

- `url`: https://docs.rs/url/latest/url/

Expected line reduction:

- Roughly 100-250 LOC

### 4. Curl HTTP status text

Current:

- `crates/just-bash/src/commands/curl/mod.rs`
- Manual `default_status_text(status: u16)`.

Replace with:

- `http::StatusCode`

Use for:

- `StatusCode::from_u16`
- `StatusCode::canonical_reason`

Why:

- Removes a manually curated status table.
- Keeps behavior typed and future-proof.

Reference:

- `http::StatusCode`: https://docs.rs/http/latest/http/status/struct.StatusCode.html

Expected line reduction:

- Small, but worthwhile

### 5. HTML to Markdown

Current:

- `crates/just-bash/src/commands/html_to_markdown_cmd.rs`
- Regex-based HTML conversion.

Replace with:

- `htmd`

Why:

- Purpose-built HTML-to-Markdown converter.
- Apache-2.0 license fits the workspace.
- Should delete a large volume of brittle regex replacement code.

Reference:

- `htmd`: https://docs.rs/htmd/latest/htmd/

Important rejection:

- Do **not** use `html2md`.
- It is GPL-3.0+, which is the wrong licensing direction for this workspace.

Expected line reduction:

- Roughly 120-220 LOC

## Existing Crates To Use More Aggressively

These do not require new dependencies, but they should replace custom logic.

### `glob`

Already present in `crates/just-bash/Cargo.toml`.

Use it to replace simple wildcard matchers in:

- `crates/just-bash/src/commands/find/matcher.rs`
- `crates/just-bash/src/commands/tar/mod.rs`
- `crates/just-bash/src/interpreter/builtins/help_cmd.rs`

Reference:

- `glob`: https://docs.rs/glob/latest/glob/

Why:

- The crate already supports Unix shell-style patterns through `Pattern`.
- This avoids adding `globset` unless profiling later proves we need compiled multi-pattern matching.

### `flate2`

Already present.

Use it more fully in:

- `crates/just-bash/src/commands/gzip/mod.rs`

Targets:

- Replace manual gzip header parsing where possible.
- Keep command semantics, but stop carrying unnecessary gzip metadata parsing code.

### `chrono`

Already present.

Use it to replace manual calendar logic in:

- `crates/just-bash/src/commands/tar/mod.rs`
- any other date/time formatting helpers that still compute leap years or timestamps manually

## Optional Follow-On Crate

### `lexopt`

Reference:

- `lexopt`: https://docs.rs/lexopt/latest/lexopt/

This is **not** a phase-1 dependency.
It is useful if the command-specific option parsers remain too large after the main swaps land.

Good candidates later:

- `crates/just-bash/src/commands/curl/parse.rs`
- `crates/just-bash/src/commands/gzip/mod.rs`
- possibly `tar` and `date`

Rule:

- Do not introduce `lexopt` until the command behavior is already stable under tests.

## Not Worth a Crate Swap

These are still cleanup targets, but not library-replacement targets.

- `crates/just-bash/src/parser/lexer.rs`
- `crates/just-bash/src/parser/parser.rs`
- `crates/just-bash/src/commands/awk/*`
- `crates/just-bash/src/commands/sed/*`
- `crates/just-bash/src/fs/in_memory_fs.rs`

These implement custom shell, language, or VFS semantics that are too repo-specific to replace safely.

## Global Rules For The Squad

1. Keep all features.
2. Keep tests green.
3. One agent owns one write-set. No overlapping edits unless explicitly listed below.
4. Only the integrator owns `Cargo.lock`.
5. Prefer deleting code to moving it.
6. If a helper becomes shared, assign a single owner for that helper.
7. No agent should reformat the entire workspace.
8. No agent should touch `icloud-*` crates unless a compile fix requires it.

## Branching And Integration Model

- One branch per agent.
- One staging branch owned by the integrator.
- Agents merge into staging only after their focused verification passes.
- Integrator resolves `Cargo.toml` and `Cargo.lock` conflicts.

Recommended branch names:

- `crate-swap/agent-00-integrator`
- `crate-swap/agent-01-checksums`
- `crate-swap/agent-02-tar-core`
- `crate-swap/agent-03-tar-surface`
- `crate-swap/agent-04-gzip`
- `crate-swap/agent-05-patterns`
- `crate-swap/agent-06-curl-types`
- `crate-swap/agent-07-curl-parse`
- `crate-swap/agent-08-html-markdown`
- `crate-swap/agent-09-find`
- `crate-swap/agent-10-fs-paths`
- `crate-swap/agent-11-interpreter-dedupe`
- `crate-swap/agent-12-parser-lexer`
- `crate-swap/agent-13-parser-driver`

## Ownership Map

### Agent 00: Integrator

Owns:

- `crates/just-bash/Cargo.toml`
- `Cargo.lock`
- `PROGRESS.md`
- final verification and merge coordination

Responsibilities:

- Land dependency additions
- Resolve cross-branch conflicts
- Run full verification
- Keep the staging branch green

No feature work beyond small integration glue

### Agent 01: Checksums

Owns:

- `crates/just-bash/src/commands/md5sum/mod.rs`

Goal:

- Replace handwritten hash implementations with RustCrypto crates

Acceptance:

- Output format unchanged
- Existing checksum tests pass
- Add standard digest vectors if current tests are too weak

Focused verification:

- `cargo nextest run -p just-bash --lib -E 'test(/^commands::md5sum::/)'`

### Agent 02: Tar Core

Owns:

- `crates/just-bash/src/commands/tar/archive.rs`

Goal:

- Replace custom tar archive encoding/parsing with a thin wrapper around `tar`

Acceptance:

- `TarEntry` abstraction still fits the rest of the command
- No handwritten tar header checksum logic remains

Focused verification:

- `cargo nextest run -p just-bash --lib -E 'test(/^commands::tar::/)'`

### Agent 03: Tar Surface

Owns:

- `crates/just-bash/src/commands/tar/mod.rs`
- `crates/just-bash/src/commands/tar/options.rs`

Depends on:

- Agent 02 for archive core
- Agent 05 if shared pattern matching lands first

Goal:

- Keep CLI semantics while deleting manual archive/time/pattern glue that the new core makes unnecessary

Acceptance:

- create/list/extract still work
- gzip-wrapped tar flows still work
- exclude semantics and directory ordering stay stable

Focused verification:

- `cargo nextest run -p just-bash --lib -E 'test(/^commands::tar::/)'`

### Agent 04: Gzip

Owns:

- `crates/just-bash/src/commands/gzip/mod.rs`

Goal:

- Use `flate2` more fully
- remove manual gzip metadata/header parsing where the crate already provides it
- split the file if necessary, but prefer deletion

Acceptance:

- `gzip`, `gunzip`, and `zcat` keep current flags and outputs
- list/test/decompress/keep/stdout behavior remains covered

Focused verification:

- `cargo nextest run -p just-bash --lib -E 'test(/^commands::gzip::/)'`

### Agent 05: Pattern Matching

Owns:

- `crates/just-bash/src/commands/find/matcher.rs`
- `crates/just-bash/src/interpreter/builtins/help_cmd.rs`
- new shared helper if needed:
  - `crates/just-bash/src/commands/utils/patterns.rs`
  - or `crates/just-bash/src/shell/simple_patterns.rs`

Do not edit:

- `crates/just-bash/src/commands/tar/mod.rs` directly

Goal:

- Replace duplicated simple glob logic with `glob::Pattern` or a single shared wrapper around it

Acceptance:

- `find -name/-path` wildcard behavior matches current tests
- builtin `help` wildcard lookup remains unchanged

Focused verification:

- `cargo nextest run -p just-bash --lib -E 'test(/^commands::find::|^interpreter::builtins::help_cmd::/)'`

### Agent 06: Curl Types / Transport

Owns:

- `crates/just-bash/src/commands/curl/mod.rs`
- `crates/just-bash/src/commands/curl/types.rs`
- `crates/just-bash/src/commands/curl/response_formatting.rs`

Goal:

- Introduce `url::Url` and `http::StatusCode` where they shrink manual logic

Acceptance:

- URL normalization, redirect handling, remote-name extraction, and status text stay correct
- No behavior regressions in verbose/include/fail modes

Focused verification:

- `cargo nextest run -p just-bash --lib -E 'test(/^commands::curl::/)'`

### Agent 07: Curl Parse / Form

Owns:

- `crates/just-bash/src/commands/curl/parse.rs`
- `crates/just-bash/src/commands/curl/form.rs`

Depends on:

- Agent 06 if `CurlOptions` changes materially

Goal:

- Replace custom percent encoding with `url::form_urlencoded`
- simplify form parsing and optionally prepare the file for later `lexopt`

Acceptance:

- combined short flags still work
- `--data-urlencode`, multipart form, headers, auth, upload, and write-out parsing stay intact

Focused verification:

- `cargo nextest run -p just-bash --lib -E 'test(/^commands::curl::/)'`

### Agent 08: HTML To Markdown

Owns:

- `crates/just-bash/src/commands/html_to_markdown_cmd.rs`

Goal:

- Replace regex-driven conversion with `htmd`

Acceptance:

- Current tests pass
- Empty input, file/stdin behavior, and help output unchanged
- Resulting markdown remains stable enough for current consumers

Focused verification:

- `cargo nextest run -p just-bash --lib -E 'test(/^commands::html_to_markdown_cmd::/)'`

### Agent 09: Find Traversal / Actions

Owns:

- `crates/just-bash/src/commands/find/mod.rs`
- may add:
  - `crates/just-bash/src/commands/find/traversal.rs`
  - `crates/just-bash/src/commands/find/actions.rs`

Do not edit:

- `crates/just-bash/src/commands/find/matcher.rs`

Depends on:

- Agent 05 only if the matcher API changes

Goal:

- Split traversal, action execution, and delete handling apart
- reduce `find/mod.rs` size without changing semantics

Acceptance:

- `-depth`, `-delete`, `-exec`, pruning, and default print behavior all preserved

Focused verification:

- `cargo nextest run -p just-bash --lib -E 'test(/^commands::find::/)'`

### Agent 10: FS Path Utilities

Owns:

- `crates/just-bash/src/fs/in_memory_fs.rs`
- may add:
  - `crates/just-bash/src/fs/path.rs`
- `crates/just-bash/src/interpreter/builtins/cd_cmd.rs`
- `crates/just-bash/src/interpreter/builtins/dirs_cmd.rs`
- `crates/just-bash/src/interpreter/builtins/source_cmd.rs`

Goal:

- Consolidate duplicated `normalize_path` logic
- keep VFS semantics stable

Acceptance:

- no change in symlink resolution or cwd behavior
- path normalization logic exists in one place

Focused verification:

- `cargo nextest run -p just-bash --lib -E 'test(/in_memory_fs|cd_cmd|dirs_cmd|source_cmd/)'`

### Agent 11: Interpreter Dedupe

Owns:

- `crates/just-bash/src/interpreter/types.rs`
- `crates/just-bash/src/interpreter/interpreter.rs`
- `crates/just-bash/src/interpreter/mod.rs`

Goal:

- Remove duplicated interpreter abstractions
- collapse overlapping type/interface definitions into one owning module

Acceptance:

- public re-exports still compile
- interpreter callers do not carry duplicate context abstractions

Focused verification:

- `cargo nextest run -p just-bash --lib -E 'test(/^interpreter::/)'`

### Agent 12: Parser Lexer Split

Owns:

- `crates/just-bash/src/parser/lexer.rs`
- any new lexer submodules under `crates/just-bash/src/parser/`

Goal:

- Split by lexical domain without changing tokenization semantics

Acceptance:

- parser lexer tests all pass
- no change to public lexer API from the outside

Focused verification:

- `cargo nextest run -p just-bash --lib -E 'test(/^parser::lexer::/)'`

### Agent 13: Parser Driver Split

Owns:

- `crates/just-bash/src/parser/parser.rs`
- possibly `crates/just-bash/src/parser/mod.rs`
- any new parser orchestration submodules

Goal:

- Split parser entry orchestration from grammar-specific behavior

Acceptance:

- parser tests all pass
- no AST behavior change

Focused verification:

- `cargo nextest run -p just-bash --lib -E 'test(/^parser::parser::/)'`

## Merge Waves

### Wave A: Immediate parallel work

These agents can start immediately with disjoint write sets:

- Agent 01
- Agent 02
- Agent 04
- Agent 05
- Agent 06
- Agent 08
- Agent 10
- Agent 11
- Agent 12
- Agent 13

### Wave B: Consumer cleanup after foundations land

- Agent 03 after Agent 02, optionally after Agent 05
- Agent 07 after Agent 06 if needed
- Agent 09 after Agent 05 if matcher interfaces changed

### Wave C: Optional parser/option-parser stretch

Only after Waves A and B are green:

- consider `lexopt` adoption in large command parsers
- consider `date` simplification using `chrono` formatting instead of manual token expansion

## Expected Net Reduction

Phase 1 realistic target:

- 1500-3000 LOC deleted with no feature loss

The biggest deletion opportunities are:

1. checksums
2. tar archive core
3. html-to-markdown
4. duplicated glob/path helpers
5. curl URL/status/encoding helpers

Parser/interpreter work will improve maintainability more than total LOC.

## Verification Matrix

Every worker must run:

```bash
cargo check -p just-bash
```

Every worker must also run the focused `nextest` filter for their module.

The integrator must run:

```bash
cargo fmt --package just-bash
cargo check -p just-bash
cargo nextest run -p just-bash --lib
cargo test -p just-bash --lib
```

Optional measurement commands:

```bash
find crates/just-bash/src -type f -name '*.rs' -print0 | xargs -0 wc -l | sort -nr | head -n 40
rg -n "fn glob_match|fn normalize_path|fn md5\\(|fn sha1\\(|fn sha256\\(" crates/just-bash/src -g '!target'
```

## Required Handoff Template For Each Agent

Each branch handoff should include:

```text
Files changed:
Dependencies requested:
Tests run:
Approximate line delta:
Behavioral parity notes:
Known risks:
```

## Final Acceptance Criteria

This plan is complete when all of the following are true:

- `cargo nextest run -p just-bash --lib` passes
- `cargo test -p just-bash --lib` passes
- all chosen crate swaps are landed
- duplicated micro-utilities targeted above are unified
- total code size is materially lower
- no iCloud-specific logic leaked into `just-bash`

## Explicitly Deferred

These are not part of this final squad plan unless everything above lands early:

- `crates/icloud-api/src/notes/markdown.rs`
- `crates/icloud-bash/src/vfs.rs`
- full awk or sed architecture rewrites
- full bash grammar replacement
