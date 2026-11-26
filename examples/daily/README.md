# daily

An agent set for everyday work: `scout`, `red`, `writer`, `critic`.

| Agent | Does | Rights |
|---|---|---|
| `scout` | collects what root needs to know | reads everything |
| `red` | writes failing tests | writes only test files |
| `writer` | makes the change, makes the tests pass | writes `crates/**` and `README.md`, never test files |
| `critic` | reviews the change | reads in full only the changed files |

`writer` and `critic` are the same agent with two contracts: they share `rules/rust.md`, and the critic starts with an
empty context because every step does.

Chains: `scout`; `writer → critic`; `scout → red → writer → critic`.

The harness, not the agent, reports which files were written (`written_as`). Agents that write work in a git
worktree copy of the project; the project itself is never changed.

To use it in a project, copy `agents/*` into `.blind/agents/` and `rules/*` into `.blind/rules/`.
