# baseline

The first agent set for smoke tests: `explorer → planner → coder → reviewer`.
Plan: [`docs/EVAL-PLAN.md`](../../../docs/EVAL-PLAN.md).

- `agents/` — one folder per agent: contract, settings, prompt.
- `rules/rust.md` — code rules; `coder` and `reviewer` list it in `agent.json` as `"rules": ["rust"]`, and the
  harness adds it after their prompt.
- `chain` — the order of the steps.

The reviewer takes `changed` and may read only those files. Today `coder` fills `changed`; the decision is that the
harness will fill it from the real diff (`docs/DESIGN.md` §5.10).
