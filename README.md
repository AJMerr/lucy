# Lucy

A model-agnostic personal AI assistant with a netrunner's attitude. Talk to her in the terminal
today; later she'll live on your desktop as an animated sprite with speech bubbles that you can
toggle on and off.

> **Status:** v0.1.0 plus tools (unreleased): terminal chat with any OpenAI-compatible or
> Anthropic model, now able to read, search, run and edit with your approval. External MCP
> servers, agent mode and the desktop sprite are next. See [release notes](CHANGELOG.md) and the
> [roadmap](docs/ROADMAP.md).

## What works today

- **Any model.** OpenAI-compatible APIs (Ollama, OpenAI, Qwen/DashScope, OpenRouter, LM Studio,
  llama.cpp, vLLM…) and Anthropic, set up as named profiles. Switch per chat with `/model`.
- **Lucy's voice, facts intact.** She stays in character, but accuracy, clean code and plain safety
  warnings come first. `/plain` drops the character entirely. The persona is a Markdown file you
  can override.
- **Streaming chat with Markdown.** Headings, lists, inline code and syntax-highlighted code
  blocks render as the reply arrives. Ctrl+C stops a reply and keeps what was said.
- **Tools, with your say-so.** Lucy can read, list and search files, fetch web pages, run
  commands, and write or edit files. Reads in your home folder just happen; anything that
  changes files, runs a command or goes online asks first (`y`, `a` for always this chat, `n`).
  Private paths like `~/.ssh` and `.env` files always ask.
- **Saved conversations.** Every chat is stored locally in SQLite. Continue the last one with
  `lucy -c`, or resume any by ID prefix.
- **Keys kept out of files.** API keys live in your OS keyring, with environment-variable
  overrides.
- **One background core.** The `lucy` command starts it on demand. It exits after 30 idle minutes,
  and future front ends (like the sprite) share it.

## Quick start

You need a Rust toolchain (`cargo`), and either a local model server or an API key.

```sh
git clone https://github.com/AJMerr/lucy && cd lucy
cargo install --path crates/lucy-cli     # installs `lucy` into ~/.cargo/bin

lucy config init                         # pick a provider and model, and tell Lucy your name
lucy auth set anthropic                  # only for providers that need a key
lucy                                     # start chatting
```

Make sure `~/.cargo/bin` is on your `PATH`. You can also skip installing and use
`cargo run -q -p lucy-cli -- <args>`.

**Local models:** install [Ollama](https://ollama.com), then run `ollama pull qwen3:14b` and pick
"Local Qwen via Ollama" in the wizard. On NixOS, set `services.ollama.enable = true;`, and use
`package = pkgs.ollama-cuda;` for NVIDIA GPUs.

## Using Lucy

```sh
lucy                       # new chat
lucy -c                    # continue your most recent chat
lucy -r 01m6ne             # resume a chat by any unique ID prefix
lucy ask "what's a flake?" # one answer, plain text, then exit
lucy ask -y "tidy todo.md" # approve every tool call (only for scripts you trust)
git diff | lucy ask        # the prompt can come from stdin
```

In the chat:

| Command | What it does |
| --- | --- |
| `/model [profile]` | list profiles, or switch this chat to one |
| `/plain [on\|off]` | drop Lucy's voice (toggles without an argument) |
| `/new`, `/sessions` | start a new chat, list saved chats |
| `/resume <id>`, `/rename <title>`, `/delete <id>` | manage saved chats |
| `/tools` | list tools and whether each runs by itself or asks |
| `/last` | show the last tool call's full input and output |
| `/mode` | show the mode (agent mode arrives in a later version) |
| `/help`, `/quit` | the list of commands, leave |

Keys: Tab completes commands and profile names, Alt+Enter (or a trailing `\`) starts a new line,
Ctrl+C stops a reply, and Ctrl+D quits. Start a message with `//` to send a literal `/`.

Other commands: `lucy config path`, `lucy auth set|remove|status`.

`lucy ask` prints the answer on stdout and tool activity on stderr. When a tool needs approval
it asks on your terminal, even if the question was piped in. With no terminal (cron, CI) it
declines, unless you pass `--yes`.

### Configuration

`lucy config init` writes `~/.config/lucy/config.toml` (or the macOS/Windows equivalent).
[`docs/config.example.toml`](docs/config.example.toml) documents every setting. Keys never go in
that file. Instead:

- `lucy auth set <key_name>` stores a key in the OS keyring (Secret Service, Keychain or Credential
  Manager).
- `LUCY_KEY_<NAME>` overrides it, and vendor variables like `ANTHROPIC_API_KEY` are used as a
  fallback.
- `lucy auth status` shows where each key comes from, without printing it.

Tools are set up under `[tools]`: turn the built-ins off, change the private-path list, or
set a tool to `auto`, `ask` or `deny`. Add `tools = false` to a profile whose model can't
call tools. See [ADR 0008](docs/adr/0008-tools-over-mcp.md).

To change Lucy's voice, copy [`persona/lucy.md`](persona/lucy.md) to
`~/.config/lucy/personas/lucy.md` and edit it. Text between `<!-- voice -->` markers is dropped in
plain mode.

### Where things live

| What | Default (Linux) | Override |
| --- | --- | --- |
| Config | `~/.config/lucy/config.toml` | `LUCY_CONFIG` |
| Chats, history, core log | `~/.local/share/lucy/` | `LUCY_DATA_DIR` |
| Core socket | `$XDG_RUNTIME_DIR/lucy/core.sock` | `LUCY_SOCKET` |

### Troubleshooting

- **Something's wrong with the core:** check `~/.local/share/lucy/core.log`.
- **You rebuilt Lucy but the old behaviour sticks:** the previous core is still running. Restart it
  with `pkill -f "lucy core run"`, and the next `lucy` starts a fresh one.
- **Keyring errors on Linux:** you need a Secret Service provider (e.g. gnome-keyring), or set
  `LUCY_KEY_<NAME>` instead.

## How it's built

```
crates/
  lucy-protocol/   JSON-RPC messages and events shared by every side
  lucy-providers/  the Provider trait + openai_compat and anthropic streaming adapters
  lucy-core/       the background core: config, keys, sessions, personas, tool rules, socket
  lucy-client/     connects front ends to the core (and starts it on demand)
  lucy-cli/        the `lucy` command: chat loop, Markdown renderer, setup wizard
  lucy-tools/      built-in tools (files, search, fetch, shell) as an in-process MCP server
apps/sprite/       Tauri desktop overlay (milestone 2)
persona/           built-in personas
packs/placeholder/ the only sprite pack tracked in git
docs/              ADRs, roadmap, example config
```

The design decisions are recorded in [`docs/adr/`](docs/adr/).

## Development

```sh
cargo test --workspace                                # everything runs offline
cargo clippy --workspace --all-targets -- -D warnings
nix develop                                           # optional; needed later for the Tauri sprite
```

Tests run against fake model servers and recorded real streams. To record a new stream:
`cargo run -p lucy-providers --example capture`. To preview the system prompt Lucy would send:
`cargo run -q -p lucy-core --example system_prompt`. CI runs fmt, clippy and the tests on Linux,
macOS and Windows.

## Sprite art

No character art is committed. Lucy loads sprite packs from your data directory
(`~/.local/share/lucy/packs/` on Linux). Copy `packs/placeholder`, swap the frames, and point
`[sprite] pack` at it. See [ADR 0005](docs/adr/0005-sprite-tauri-and-packs.md).

## Licence

All rights reserved for now. A licence will be chosen before the repo goes public.
Lucy is a fan-inspired name; this project is not affiliated with CD Projekt or Studio Trigger.
