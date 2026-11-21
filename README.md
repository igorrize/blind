# blind

A TUI harness for AI agents with super-strict limits on agents and sub-agents.

- The harness runs the agent loop itself, with any model over API (Anthropic, OpenAI-compatible, local).
- Coordinators can only call other agents. Workers can only use the tools their contract grants,
  inside an OS sandbox, writing only to a throwaway copy.
- Rights only shrink down the call tree. Only a human writes and approves the rules.

Status: **in progress** — strict JSON contracts, chain type checks, `blind agent check`, an OpenAI-compatible
provider (ollama.com), a chat with root in the TUI, workers under a contract (`blind run`) and chains of them (`blind chain run`). Workers can find, list, read and search project files (search runs on the fff engine) within the read rights of their contract; they cannot write or run commands yet. Design: [`docs/DESIGN.md`](docs/DESIGN.md).

## Build

```sh
cargo blind                                         # chat with root, Ctrl-C to quit (needs the config below)
cargo blind agent check --dir examples/agents       # check agent folders
cargo test
```

## Talk to a model

Create `~/.config/blind/providers.toml`:

```toml
[defaults]
model = "ollama-cloud/glm-5.3"

[providers.ollama-cloud]
kind = "openai-compatible"
base_url = "https://ollama.com/v1"
api_key = { env = "OLLAMA_API_KEY" }
```

Then:

```sh
export OLLAMA_API_KEY=...
cargo blind models                                  # model names the provider offers
cargo blind ask "Say hello in one sentence."
cargo blind ask "..." --model ollama-cloud/glm-5.3-flash
cargo blind run planner --dir examples/agents --task "Add a --json flag to agent check"
cargo blind chain check explorer reviewer fixer --dir examples/agents
cargo blind chain run explorer reviewer --dir examples/agents --project . --task "Review the contract loader"
```

The key is read from the environment only and never written to a file.

Keys in the TUI: `Enter` sends, `Ctrl-X 1` opens the root chat, `Ctrl-X 2` opens settings, `Ctrl-X s` focuses the sidebar (`↑↓`, `Enter`, `Esc`) (`↑↓` and `Enter`
pick the theme and the root model, `r` reloads the model list), `Ctrl-C` quits. The theme is one of the four Catppuccin flavors and is saved to `~/.config/blind/ui.toml`.

`cargo blind` is an alias from `.cargo/config.toml`. To get a plain `blind` command:

```sh
cargo install --locked --path crates/blind
```

`--locked` is required: the file search engine builds only with the dependency versions pinned in `Cargo.lock`.

## Layout

| Crate | Role |
|---|---|
| `blind-core` | pure core: contracts, grants, task lifecycle — no IO |
| `blind` | the app: kernel, providers, MCP, TUI |
| `blind-exec` | sandboxed executor for workers — no HTTP client |
