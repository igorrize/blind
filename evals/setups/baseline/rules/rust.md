# Code rules

1. No comments in code. Names say what the code does; when a piece needs explaining, extract a function with a
   telling name instead.
2. Small functions, one job each.
3. One TUI component per file under `crates/blind/src/tui/`. Screen code does not reach the providers, the worker
   loop, the agent loader or the file system: `crates/blind/tests/architecture.rs` checks this.
4. A `match` over an enum lists every variant; no `_ =>` arm. Clippy `wildcard_enum_match_arm` is denied.
5. No `unsafe`. `cargo fmt --check` and `cargo clippy --all-targets -- -D warnings` pass.
6. Tests live next to the code in `#[cfg(test)] mod tests`. A test name states the behaviour as a sentence.
7. Errors that can happen at run time are returned, not unwrapped: `thiserror` in libraries, `anyhow` at the edges.
8. `blind-core` stays pure: no IO, and no dependencies besides `serde`, `serde_json` and `thiserror`.
9. A new dependency only when it is needed, with an exact version, and the reason goes into the summary.
