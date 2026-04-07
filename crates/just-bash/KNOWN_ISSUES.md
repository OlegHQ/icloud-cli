# Known Issues

## awk (awk-rs crate)

The `awk` command is backed by the [`awk-rs`](https://crates.io/crates/awk-rs) crate (v0.1).
Two features work differently from a full POSIX awk:

### ORS (Output Record Separator) is ignored

`awk-rs` 0.1 does not honour custom `ORS`. Setting `ORS="|"` still produces
newline-separated output. OFS works correctly.

Tracked by ignored test: `commands::awk::tests::test_ors_known_issue`

### ENVIRON reads from the host process, not the sandbox env

`awk-rs` populates `ENVIRON` from `std::env` at runtime. It has no API to
inject a custom environment map, so `ENVIRON` reflects the real process
environment rather than the sandbox's `CommandContext.env`.

Tracked by ignored test: `commands::awk::tests::test_environ_sandbox_known_issue`

## sed (sed-rs crate)

The `sed` command is backed by the [`sed-rs`](https://crates.io/crates/sed-rs) crate (v1.0.0).

### `r`/`w`/`R`/`W` commands use the host filesystem, not the sandbox

`sed-rs` performs file I/O through `std::fs` internally. The `r` (read file),
`w` (write to file), `R` (read line), and `W` (write first line) commands
bypass `InMemoryFs` and hit the real filesystem, where the referenced paths
typically do not exist. This means `r` silently produces no output and `w`
either fails or writes to the host disk.

Tracked by ignored tests:
- `commands::sed::tests::test_read_file_command`
- `commands::sed::tests::test_write_file_command`
