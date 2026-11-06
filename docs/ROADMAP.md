# Roadmap

## Milestone 0 — Repo scaffold ✅

Cargo workspace, Nix dev shell, CI on Linux/macOS/Windows, ADRs, persona, placeholder pack,
`Provider` trait + `mock` provider.

## Milestone 1 — Terminal Lucy

Built test-first against the `mock` provider.

1. ✅ **Config** — load `config.toml` (profiles, defaults, agent settings) from the platform config
   dir; first-run wizard writes it.
2. ✅ **Secrets** — keyring storage, `LUCY_KEY_*` env override, `lucy auth set|remove|status`.
3. ✅ **Providers** — `openai_compat` (streaming) and `anthropic` (streaming), with recorded-fixture
   tests. Qwen preset in the wizard. *Fixtures are hand-written from the documented formats until
   real captures replace them (`cargo run -p lucy-providers --example capture`).*
4. ✅ **Sessions** — SQLite store; create, append, list, load (resume), rename, delete, ID-prefix lookup.
5. ✅ **Persona** — built-in personas plus user overrides in `<config dir>/lucy/personas/`; system
   prompt with voice/plain blocks and live context (date, machine, name, model). `/plain` command
   itself lands with the CLI (step 7).
6. ✅ **Core + IPC** — core process, JSON-RPC over local socket, spawn-on-demand from the CLI,
   idle exit, live events to subscribers. Plain-text `lucy ask` works end to end.
7. ✅ **CLI** — `lucy` interactive streaming chat with markdown rendering and `/model`, `/mode`,
   `/plain`, `/sessions`; `lucy ask "..."` one-shot (reads stdin).
8. ✅ **Tools** (ADR 0008)
   - ✅ **8a** — tool calls in both adapters; tool loop in the core; Assist-mode rules and
     approvals (y / always / no); built-in tools (`read_file`, `list_dir`, `search_files`,
     `fetch_url`, `run_command`, `write_file`, `edit_file`) as an in-process MCP server;
     `/tools`, `/last`, `lucy ask --yes`.
   - ✅ **8b** — external MCP servers started as programs (shared, restarted on crash, tokens
     from the keyring); tools named `<server>__<tool>`; `lucy mcp add|list|remove|serve`.
   - ✅ **8c** — remote MCP servers by URL (streamable HTTP), with keyring API keys or OAuth
     sign-in (`lucy mcp login|logout`, tokens in the keyring, refreshed automatically).
9. **Agent mode** — `AgentLevel` enforcement, step and cost caps.
10. **Service** — `lucy service install|uninstall`.
11. **Optional HTTP/WebSocket** transport (off by default).

## Milestone 2 — Desktop sprite

Tauri app in `apps/sprite`: transparent always-on-top overlay, frames renderer, `pack.toml`
loader, speech bubbles, hotkey/click input, full chat window for long replies, Hyprland window
rule docs, toggle keybind.

## Later

- Long-term memory (editable file of facts about the user)
- Live2D renderer
- Voice: local Whisper STT, pluggable TTS, lip-sync to the `talking` state
- More providers (Gemini, ...)
- Decide licence before making the repo public
