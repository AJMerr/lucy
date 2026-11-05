# Lucy

A model-agnostic personal AI assistant with a netrunner's attitude. Talk to her from the
terminal today; later she'll live on your desktop as an animated sprite with speech bubbles
that you can toggle on and off.

> **Status:** Milestone 0 (scaffold). Nothing user-facing works yet. See [ROADMAP](docs/ROADMAP.md).

## What she'll do

- **Any model** — OpenAI-compatible APIs (OpenAI, Ollama, Qwen, OpenRouter, LM Studio...) and
  Anthropic, configured as named profiles you switch with `/model`.
- **Two modes** — *Assist* (chat + MCP tools, asks before risky actions) and *Agent*
  (autonomous multi-step tasks with configurable freedom and cost caps).
- **Remembers conversations** — resumable sessions stored locally.
- **Desktop sprite** (milestone 2) — a Tauri overlay that renders any user-supplied sprite pack.
- **Cross-platform** — Linux, macOS, Windows.

## Layout

```
crates/
  lucy-protocol/   shared IPC types
  lucy-providers/  Provider trait + adapters (mock, openai_compat, anthropic)
  lucy-core/       background service: sessions, tools, modes, IPC
  lucy-cli/        the `lucy` binary
apps/sprite/       Tauri desktop overlay (milestone 2)
persona/           system-prompt personas
packs/placeholder/ the only sprite pack tracked in git
docs/              ADRs, roadmap, example config
```

## Development

```sh
nix develop          # or: install a Rust toolchain yourself
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

CI runs fmt, clippy and tests on Linux, macOS and Windows.

## Sprite art

No character art is committed. Lucy loads sprite packs from your data directory
(`~/.local/share/lucy/packs/` on Linux). Copy `packs/placeholder`, swap the frames, and point
`[sprite] pack` at it. See [ADR 0005](docs/adr/0005-sprite-tauri-and-packs.md).

## Licence

All rights reserved for now. A licence will be chosen before the repo goes public.
Lucy is a fan-inspired name; this project is not affiliated with CD Projekt or Studio Trigger.
