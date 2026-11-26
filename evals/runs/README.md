# Runs

One folder per run: `<date>-<setup>-<who ran it>/`. A run is one scenario played by one agent set; runs of the same
scenario are compared by their `results.json`.

| File | What it holds |
|---|---|
| `NN-<agent>.input.json` | what the harness handed to the step |
| `NN-<agent>.output.json` | the accepted answer of the step |
| `NN-<agent>.rejected-K.json` | an answer the harness rejected, with the reason in `results.json` |
| `patch.diff` | every change the agents made, against the base commit |
| `checks.txt` | format, lint, tests and acceptance tests after the chain |
| `results.json` | the numbers, in the shape below |
| `report.md` | what happened, in words, and what to fix in the scenario, the agents or the harness |

## `results.json`

```json
{
  "scenario": "copy-blocks",
  "setup": "baseline",
  "base": "caeedbb",
  "runner": "claude-code",
  "steps": [
    {
      "agent": "explorer",
      "model": "sonnet",
      "attempts": 3,
      "rejections": ["`notes` is 4347 characters, at most 3000 allowed"],
      "tool_calls": 10,
      "seconds": 96,
      "tokens": 68322
    }
  ],
  "checks": {
    "fmt": true,
    "clippy": true,
    "tests": { "passed": 0, "failed": 0 },
    "acceptance": { "passed": 0, "failed": 0 }
  },
  "patch": { "files": 0, "added": 0, "removed": 0, "comment_lines": 0 },
  "judged": { "works_in_terminal": null, "rules_kept": null, "clipboard_decision": null }
}
```

## What is compared

Runs are compared by their result, in this order:

1. `acceptance` — the hidden acceptance tests pass;
2. `checks` — format, lint and the project's own tests pass;
3. `judged` — a human tries the change and rates it from 1 to 5: does it work in the terminal, are the rules kept,
   is the clipboard decision sound;
4. the reviewer's findings, read against `patch.diff`.

The numbers in `steps` — attempts, rejections, tool calls, seconds, tokens — only explain a result; they are not
compared. Tokens are not even comparable between runners: Claude Code reports one number per sub-agent, `blind`
sums every turn.
