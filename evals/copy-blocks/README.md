# copy-blocks

Smoke-test scenario: copy the last answer, or the last prompt with its answer, from the root chat.
Plan: [`docs/EVAL-PLAN.md`](../../docs/EVAL-PLAN.md).

| | |
|---|---|
| Task | [`task.md`](task.md) |
| Base commit | `64e0bbb` until the scenario is approved and tagged |
| Acceptance tests | `acceptance/` — not written yet (plan step 2) |

## Checks

Run in the copy after the agents finish:

```sh
cargo fmt --all --check
cargo clippy --all-targets -- -D warnings
cargo test --all
```

Then the acceptance tests are added to the copy and `cargo test --all` runs again.

## Judged by a human

- Copying works in a real terminal, inside zellij.
- The code follows the project rules that commands cannot check.
- The clipboard decision is explained and reasonable.
