# Changelog

## Unreleased

### Tools (step 8a)

- **Lucy can use tools.** Built in: `read_file`, `list_dir`, `search_files`, `fetch_url`,
  `run_command`, `write_file` and `edit_file`. They work in the folder you started `lucy` in,
  and need nothing else installed. They're served over MCP in-process, so external MCP servers
  (next) will follow the same rules.
- **Approvals.** Reads in your home folder run by themselves. Private paths (`~/.ssh`, `.env`
  files, keys, cloud credentials) and anything outside home ask first, even through a symlink.
  So does anything that writes files, runs commands or goes online. Answer `y`, `a` (always,
  for this chat) or `n`. Edits and writes show a diff first. Unanswered prompts are declined
  after 5 minutes.
- **In the chat**, each tool call is one line (`⚙ read_file  notes.txt  ✓ 73 B`). `/tools`
  lists tools and whether they run or ask, and `/last` shows the last call's full input and
  output. Resumed chats show past tool calls.
- **`lucy ask`** asks on your terminal even when the question is piped in, and declines when
  there's no terminal. `--yes` approves everything. Tool activity goes to stderr.
- **Config:** `[tools]` (built-ins on/off, round limit, private paths, per-tool
  `auto`/`ask`/`deny`), and `tools = false` per profile.
- Lucy's persona now says to use tools instead of guessing, and never to claim she ran or read
  something without a tool result.

### MCP servers (step 8b)

- **Use any MCP server.** `lucy mcp add github --key GITHUB_PERSONAL_ACCESS_TOKEN=github -- npx
  -y @modelcontextprotocol/server-github` adds a server, or run `lucy mcp add` alone to answer
  a few questions. It keeps your config's comments, then starts the server once and lists its
  tools. `lucy mcp list` shows which servers start, and `lucy mcp remove` takes one out.
- Servers start the first time a chat needs tools, stay up for every chat, and restart on next
  use if they crash or their config changes. Removed servers stop on the next message.
- **Tokens stay in the keyring:** `env_keys = { VAR = "keyname" }` fills a variable from
  `lucy auth set keyname` (or `LUCY_KEY_KEYNAME`). `lucy auth status` lists those keys too.
- Their tools appear as `<server>__<tool>` and follow the same approval rules as the built-ins.
  `[tools.rules]` can set a whole server to `ask` or `deny`.
- `/tools` and `lucy mcp list` show servers that failed to start, and why.
- `lucy mcp serve` offers Lucy's built-in tools to other MCP clients over stdio.

### Remote MCP servers (step 8c)

- **Servers at a URL.** `lucy mcp add linear --url https://mcp.linear.app/mcp`; or type the
  URL when `lucy mcp add` asks for the command.
- **Sign-in (OAuth).** If a server wants one, `add` offers to log in. `lucy mcp login <name>`
  opens your browser and catches the redirect on localhost. Lucy registers herself
  automatically, or uses `oauth_client_id` for an app you registered.
- **Tokens:** kept in the keyring and refreshed when they expire. If a refresh is refused,
  Lucy says to log in again. `lucy mcp logout <name>` forgets a sign-in.
- **API keys instead:** `--bearer-key keyname` (sent as `Authorization: Bearer`), or
  `--header-key HEADER=keyname`, from the keyring. Plain `--header`s work too. A rejected key
  names the keyring entry to fix.
- A remote server's read-only tools run by themselves. Their paths aren't on your machine, so
  the home-folder checks don't apply.

### Upgrading

- **Protocol v2.** Stop the old core after installing (`pkill -f "lucy core run"`); the new
  `lucy` says so if you forget.
- The session database migrates itself (it adds tool-call columns).
- Ollama's default 4,096-token context fills faster with tools. Raise `OLLAMA_CONTEXT_LENGTH`
  (e.g. 16384) if replies lose track.

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

- **Tested day to day on Linux only.** Every test, including starting the background core, passes
  in CI on Linux, macOS and Windows. Interactive use has so far only happened on NixOS.
- **No tools yet.** Lucy can't read files, run commands or browse; that's next (MCP tools, step 8).
  `/mode agent` is a placeholder.
- **Some settings reset with the core.** Per-chat `/model` and `/plain` choices last until the core
  restarts (after 30 idle minutes); they aren't saved.
- **Short Ollama context.** Ollama's default window is 4,096 tokens, which long chats outgrow. Raise
  it with `OLLAMA_CONTEXT_LENGTH` (or `services.ollama.environmentVariables` on NixOS).
- **Restart the core after upgrading.** A core started by an older build keeps running until idle.
  Restart it with `pkill -f "lucy core run"` (on Windows, end the `lucy.exe` process running
  `core run`).
- **No desktop sprite or voice yet.**

### Under the hood

Rust workspace with five crates (protocol, providers, core, client, CLI). It has 106 automated
tests, plus one opt-in test against the real OS keyring. They include fake model servers that
stream in tiny chunks and a real recorded Qwen stream. CI runs on Linux, macOS and Windows. The
design decisions are in `docs/adr/0001`–`0007`.
