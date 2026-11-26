# copy-blocks

Smoke-test scenario: copy the last answer, or the last prompt with its answer, from the root chat.
Plan: [`docs/EVAL-PLAN.md`](../../docs/EVAL-PLAN.md).

| | |
|---|---|
| Task | [`task.md`](task.md) |
| Base commit | `caeedbb` on branch `eval/copy-blocks-reference` (a snapshot of the work in progress); a tag once the scenario is approved |
| Acceptance tests | [`acceptance/acceptance.rs`](acceptance/acceptance.rs) — hidden from the agents |
| Agent set | [`evals/setups/baseline`](../setups/baseline) |

## Checks

Run in the copy after the agents finish:

```sh
cargo fmt --all --check
cargo clippy --all-targets -- -D warnings
cargo test --all
```

Then the acceptance tests are added to the copy and `cargo test --all` runs again:

```sh
mkdir -p crates/blind/src/conversation
cp <blind>/evals/copy-blocks/acceptance/acceptance.rs crates/blind/src/conversation/acceptance.rs
printf '\n#[cfg(test)]\nmod acceptance;\n' >> crates/blind/src/conversation.rs
cargo test --all
```

They fail to compile on the base commit, because `last_answer` and `last_exchange` do not exist there.

## Judged by a human

- Copying works in a real terminal, inside zellij.
- The code follows the project rules that commands cannot check.
- The clipboard decision is explained and reasonable.
