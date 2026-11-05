# Changelog

## v0.1.0: first working version (2026-11-05)

Lucy's first usable release: a terminal assistant that works with any OpenAI-compatible or
Anthropic model, talks in Lucy's voice, and remembers your conversations.

### Highlights

- **Interactive chat.** `lucy` opens a streaming chat with line-by-line Markdown rendering and
  syntax-highlighted code blocks. It also has input history, Tab completion and multi-line input.
  Ctrl+C stops a reply without losing what was said.
- **Bring your own model.** Profiles for Ollama, OpenAI, Qwen (DashScope), OpenRouter, LM Studio,
  llama.cpp, vLLM and Anthropic. A first-run wizard sets one up, and `/model` switches a chat to
  another. Qwen's reasoning and Claude's thinking blocks are kept out of the reply.
- **Lucy's persona.** She's in character by default, under rules that keep facts accurate, code
  clean and safety warnings plain. `/plain` removes the character, and personas can be overridden
  per user. Her system prompt knows the date, your machine, your name and which model is
  answering.
- **Saved sessions.** Conversations are stored locally in SQLite, with titles taken from your first
  message. Resume them with `lucy -c`, `lucy -r <id prefix>` or `/resume`. Rename and delete with
  `/rename` and `/delete`.
- **One-shot answers.** `lucy ask "…"`, or pipe a prompt in, for scripts.
- **API keys in the OS keyring.** `lucy auth set|remove|status`, with `LUCY_KEY_*` and vendor
  environment variables as overrides.
- **Background core.** Started on demand over a local socket, shared by every front end, and exits
  after 30 idle minutes. Replies are broadcast live to subscribed clients, ready for the desktop
  sprite.

### Requirements

- A Rust toolchain to build (`cargo install --path crates/lucy-cli`).
- A model: a local server such as Ollama (tested with `qwen3:14b` on an RTX 5070), or an API key.
- On Linux, a Secret Service provider (e.g. gnome-keyring) if you want keys in the keyring.

### Known limitations

- **Windows is not yet verified end to end.** CI builds and runs most tests on Windows, but the
  test where `lucy` starts the background core still fails there and is under investigation.
  Linux and macOS pass.
- **No tools yet.** Lucy can't read files, run commands or browse; that's next (MCP tools, step 8).
  `/mode agent` is a placeholder.
- **Some settings reset with the core.** Per-chat `/model` and `/plain` choices last until the core
  restarts (after 30 idle minutes); they aren't saved.
- **Short Ollama context.** Ollama's default window is 4,096 tokens, which long chats outgrow. Raise
  it with `OLLAMA_CONTEXT_LENGTH` (or `services.ollama.environmentVariables` on NixOS).
- **Restart the core after upgrading.** A core started by an older build keeps running until idle.
  Restart it with `pkill -f "lucy core run"`.
- **No desktop sprite or voice yet.**

### Under the hood

Rust workspace with five crates (protocol, providers, core, client, CLI). It has 106 automated tests (plus one opt-in test against the real OS keyring),
including fake model servers that stream in tiny chunks and a real recorded Qwen stream. CI runs
on Linux, macOS and Windows. The design decisions are in `docs/adr/0001`–`0007`.
