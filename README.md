# Lucy

A personal AI assistant that works with any model, with a netrunner's attitude. Talk to her in
a terminal, in her app, or through her **sprite**: a small animated figure who lives on your
desktop and answers in speech bubbles.

> **Status:** v0.10.0. Lucy chats with any OpenAI-compatible or Anthropic model; you add one in a few clicks, choosing from the models the provider offers.
>
> She can:
> - read, search, run and edit with your approval;
> - use any MCP server you add;
> - work through tasks on her own, within limits you set;
> - remember you across chats;
> - remind you of things and run scheduled tasks;
> - talk and listen in her app;
> - see and make pictures.
>
> Long chats are summarised to fit. The core can run from login and serve your other machines. Her app gives you the sprite, a window for your chats and every setting, and a tray icon. Installers, a Nix flake and a home-manager module are available. See the [release notes](CHANGELOG.md).

**Documentation:** [ajmerr.github.io/lucy](https://ajmerr.github.io/lucy/) (the same guides
are in [`docs/`](docs/index.md)).

## What she does

- **Any model.** Run one on your own computer (Ollama, LM Studio, llama.cpp…) or use a hosted
  one (OpenRouter, Anthropic, OpenAI, Qwen…). Pick a provider, paste its key, choose from the
  models it offers, and switch models per chat. Long chats are summarised to fit the model.
  [Models](docs/models.md)
- **Your computer, with your say-so.** She reads and searches files, runs commands, edits
  code and fetches pages. Anything that changes something asks first, and **Undo** puts files
  back. [Tools](docs/tools.md)
- **Agent mode.** Give her a task and she works through it on her own, at a freedom level you
  choose, within step and cost caps. [Agent mode](docs/tools.md#agent-mode)
- **More tools.** Add MCP servers for Gmail and Calendar, GitHub, a web browser and hundreds
  more, from ready-made entries or a pasted command. [MCP servers](docs/mcp.md)
- **She remembers you.** She keeps what you tell her about yourself across chats, and asks
  before keeping anything sensitive. She never keeps secrets. Incognito chats remember nothing.
  [Memory](docs/memory.md)
- **Reminders and scheduled tasks.** Try "Remind me at 5 to call Mom", or "every weekday at 8,
  summarise my email". [Reminders and tasks](docs/tasks.md)
- **Voice.** Hold the mic, or say "Hey Lucy", and she answers aloud. Hearing runs on your
  computer; her voice can be local or hosted. [Voice](docs/voice.md)
- **Pictures.** Show her a picture, or ask her to draw. She uses OpenRouter image models or
  ComfyUI on your own GPU. [Pictures](docs/images.md)
- **Everywhere you are.** The terminal, the app's window, the sprite, and your other machines
  share the same chats. [The app](docs/app.md) · [Remote access](docs/remote.md)
- **Private by default.** Chats and memory stay on your machine, and API keys stay in your OS
  keyring. `/plain` drops the character whenever you want straight answers.

## Install

| | |
| --- | --- |
| **macOS** | `brew install --cask ajmerr/lucy/lucy`, or the `.dmg` from [Releases](https://github.com/AJMerr/lucy/releases/latest) |
| **Windows** | the `.msi` from [Releases](https://github.com/AJMerr/lucy/releases/latest) |
| **Arch** | `yay -S lucy-bin` |
| **Debian, Ubuntu, Fedora** | the `.deb` or `.rpm` from [Releases](https://github.com/AJMerr/lucy/releases/latest), or the `.AppImage` |
| **Nix** | `nix profile install github:AJMerr/lucy`, or the [home-manager module](docs/nix.md) |
| **From source** | `cargo install --locked --git https://github.com/AJMerr/lucy lucy-cli` |

The installers aren't signed with paid certificates, so macOS and Windows warn the first time
you open Lucy. [Installing](docs/install.md) shows how to get past that.

You also need a model: [Ollama](https://ollama.com) with `ollama pull qwen3:14b` runs one on your
own computer, or you can use an API key from OpenRouter, Anthropic or OpenAI.

## First steps

Open Lucy's app and it walks you through setup. In a terminal:

```sh
lucy config init     # pick a provider and model, and tell Lucy your name
lucy                 # start chatting
```

Then see [the first five minutes](docs/getting-started.md).

| In a chat | |
| --- | --- |
| `/model [name] [--default]` | list models, or switch this chat (and the default) |
| `/mode agent`, `/level 0-3` | let her work on her own, and how freely |
| `/clear`, `/compress` | start her afresh, or summarise the chat so far |
| `/image <description>` | make a picture |
| `/plain`, `/undo`, `/help` | no character, put files back, everything else |

## How it's built

```
crates/
  lucy-protocol/   JSON-RPC messages and events shared by every side
  lucy-providers/  the Provider trait, the OpenAI-compatible and Anthropic adapters, model lists
  lucy-core/       the background core: config, keys, chats, memory, schedules, tools, MCP
  lucy-client/     connects front ends to the core (and starts it on demand)
  lucy-cli/        the `lucy` command: chat, setup, and managing everything else
  lucy-tools/      built-in tools (files, search, fetch, shell) as an in-process MCP server
  lucy-packs/      sprite packs: loading and checking pack.toml and its frames
apps/desktop/      the desktop app's Rust side (Tauri; its own Cargo workspace), with voice
apps/ui/           its web pages (the sprite and the main window), in Svelte
persona/           built-in personas
packs/placeholder/ the built-in sprite pack
nix/               the flake's packages and home-manager module
docs/              the guides (the docs site), design decisions, example config
```

Everything talks to one background **core** over JSON-RPC: a local socket, or a WebSocket for
other machines. The design decisions are recorded in [`docs/adr/`](docs/adr/).

## Contributing

Bug reports, fixes and ideas are welcome. [CONTRIBUTING.md](CONTRIBUTING.md) covers setting up
(`nix develop` has everything) and the checks:

```sh
cargo test --workspace          # everything runs offline, against fake model servers
cd apps/ui && npm test
```

Security problems go through [private reporting](SECURITY.md), not issues. Please follow the
[code of conduct](CODE_OF_CONDUCT.md).

## Licence

Lucy is licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or
[MIT license](LICENSE-MIT), at your option. Unless you say otherwise, any contribution you submit
for inclusion in Lucy, as defined in the Apache-2.0 licence, is dual licensed as above, without
any additional terms or conditions.

Lucy is a fan-inspired name; this project is not affiliated with CD Projekt or Studio Trigger.
