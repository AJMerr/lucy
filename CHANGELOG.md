# Changelog

## Unreleased

### Desktop sprite (preview)

- **Lucy on your desktop.** `lucy sprite` starts her: a small transparent figure in the
  bottom-right corner, above your windows, animating from her sprite pack. Click her and she's
  happy. `lucy sprite quit` sends her off. Build her from `apps/sprite` (see its README); she
  doesn't talk yet.
- **No window rules needed** on Wayland compositors with layer shell (Hyprland, Sway, KDE and
  others); elsewhere she's an always-on-top window.
- **Sprite packs:** `[sprite] pack` and `scale` now work. Packs live in
  `~/.local/share/lucy/packs/`, with `placeholder` built in. A broken pack is explained in
  `sprite.log` and replaced by the placeholder.

## v0.3.0: remote access (2026-11-07)

Lucy's core can now serve other machines: your laptop's `lucy` can use your desktop's core,
with its chats, models, keys and MCP servers, over WebSocket. It's off by default, behind a
token, and capped so a leaked token can't run commands unapproved. This completes
milestone 1.

### Remote access

- **Use Lucy from another machine.** Turn on `[remote]` on the machine that runs the core and
  get its token with `lucy remote token`. Then, on the other machine, `lucy remote connect
  desktop:7077`: `lucy`, `lucy ask` and `lucy agent` there talk to that core, with its chats,
  models, keys and MCP servers. `lucy remote disconnect` goes back. `--remote <url>` or
  `LUCY_REMOTE` does it for one run. The other machine needs no config or keys.
- **Off by default, and careful when on.** Every connection needs the token (in the keyring;
  `--rotate` replaces it). Listening beyond 127.0.0.1 needs `allow_network = true`. There's no
  TLS, so use Tailscale, an SSH tunnel, or a TLS proxy (`wss://` works).
- **Remote chats work on the core's machine**, in `[agent] workspace` (else its home folder),
  and the chat banner says so. They run at most at agent level 1 (`[remote] max_level`), so a
  leaked token can't run commands unapproved. A remote client can't stop the core.
- It's the same JSON-RPC as the local socket, over WebSocket at `/rpc`, so scripts and future
  web front ends can use it too. Browsers can send the token as a `lucy-token.<token>`
  subprotocol.

### Security

- **On Windows, the core's named pipe now admits only your user** (and SYSTEM); before, other
  accounts on the same PC could connect to it.

### Upgrading from v0.2.0

- Install with `cargo install --path crates/lucy-cli`, then restart the core:
  `lucy service restart`, or `lucy core stop` if you don't use the service. The protocol is
  still v4, so the new `lucy` won't replace the old core by itself.
- Nothing to change in your config: `[remote]` stays off until you turn it on (see
  `docs/config.example.toml`). Turning it on, or changing it, needs a core restart too.

### Known limitations

- **No TLS of its own.** Remote traffic is plain WebSocket; use Tailscale, an SSH tunnel or a
  TLS proxy for anything beyond localhost.
- **One token per core.** Rotating it disconnects every other machine at their next
  connection; there are no per-device tokens yet.
- **Remote access only lasts while the core runs**, and other machines can't start it, so
  install the login service on the machine that serves.
- **Tested day to day on Linux only.** Everything passes in CI on Linux, macOS and Windows.
- The v0.2.0 limitations still apply (Windows console flash at login, level 2 commands,
  `/undo` scope, Ollama context), and there's no desktop sprite or voice yet (milestone 2).

### Under the hood

The local socket and WebSocket clients share one request handler, and `lucy-client` speaks
both, so the desktop sprite can use either. 222 automated tests, plus the opt-in keyring test:
new ones cover the WebSocket transport, its token and limits against a real core, and the
real `lucy` binary driving a remote core.

## v0.2.0: tools, agent mode and a login service (2026-11-07)

Lucy can now act, not just talk. She reads, searches, runs and edits with your approval,
uses any MCP server you add (local or remote, with OAuth sign-in), and works through whole
tasks on her own at a freedom level you choose, within step and cost caps, with `/undo` for
her file changes. Her core can also run from login, ready for the desktop sprite.

### Tools

- **Lucy can use tools.** Built in: `read_file`, `list_dir`, `search_files`, `fetch_url`,
  `run_command`, `write_file` and `edit_file`. They work in the folder you started `lucy` in,
  and need nothing else installed. They're served over MCP in-process, so external MCP servers
  follow the same rules.
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

### MCP servers

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

### Remote MCP servers

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

### Agent mode

- **Lucy can work through tasks on her own.** `/mode agent` in a chat, or `lucy agent "task"`
  from the terminal. `/level 0-3` sets how freely she works:
  - 0 asks for everything.
  - 1 (default) changes files in your folder by itself.
  - 2 runs anything local in `[agent] workspace`, commands included.
  - 3 runs everything except private paths.
- **Caps:** each task stops at `max_steps`, and at `max_cost_usd` (using `price_in`/`price_out`
  per profile; local models are free) or `max_tokens` for unpriced models. At a cap she sums
  up what's done and what's left; say "continue" to carry on. `lucy agent` exits with 2.
- **`/undo`** restores the files the last reply changed (and deletes ones it created). Repeat
  it to step further back.
- Replies that took several steps end with `· 7 steps · 12.3k tokens · $0.02`.
- **Chats remember their model, plain mode, mode and level** across core restarts.
- Token usage is now read from both providers' streams (`include_usage` for OpenAI-compatible
  servers).

### Login service

- **`lucy service install`** keeps Lucy's core running from login: a systemd user unit on
  Linux, a launchd agent on macOS, a `Run` registry value on Windows (no admin rights). It
  starts the core right away, and restarts it if it crashes.
- `lucy service status` shows whether the service is installed, how the core is running
  (service or on demand), its version and PID, and any chat it's busy with.
  `lucy service restart` picks up a new build, and `lucy service uninstall` goes back to on
  demand.
- **`lucy core stop`** stops whichever core is running. If Lucy is mid-reply it asks first
  (`--force` cancels the reply, keeping what she'd said).
- The service gets the `PATH` and `LUCY_*` settings of the shell you install it from, so tools
  find the same programs. API keys in environment variables aren't copied; keep them in the
  keyring.
- **Upgrades restart the core by themselves.** A newer `lucy` that finds an older, idle core
  stops it and starts its own, through the service if you use one.

### Upgrading from v0.1.0

- Install with `cargo install --path crates/lucy-cli`, then stop the old core once:
  `pkill -f "lucy core run"` (on Windows, end the `lucy.exe` running `core run`). The new
  `lucy` says so if you forget. From this release on, a new `lucy` replaces an idle older core
  by itself.
- Protocol v4 (v2 brought tools, v3 agent mode, v4 stopping the core).
- The session database migrates itself (it adds tool-call columns, chat settings and undo
  backups). Your config needs no changes; the new sections are optional, see
  `docs/config.example.toml`.

### Known limitations

- **Tested day to day on Linux only.** Everything passes in CI on Linux, macOS and Windows,
  and the login service has been installed for real on NixOS. On macOS and Windows neither the
  tools nor the service have been used day to day yet.
- **Windows login:** a console window may flash briefly when the service starts the core.
- **Level 2 runs commands without asking.** A command can do anything a shell can, including
  installing software or downloading, so use level 2 only in workspaces you'd let a script
  loose in.
- **`/undo` covers `write_file` and `edit_file` only**, not changes made through `run_command`
  or other servers' tools.
- **Short Ollama context.** Ollama's default 4,096-token window fills faster with tools. Raise
  `OLLAMA_CONTEXT_LENGTH` (e.g. 16384) if replies lose track.
- **No desktop sprite or voice yet** (milestone 2), and no HTTP/remote access to the core.

### Under the hood

Six crates now: `lucy-tools` holds the built-in tools as an MCP server, and MCP support comes
from `rmcp`. The permission rules are one pure function (`lucy_core::policy`). The suite has
206 automated tests, plus the opt-in keyring test. They include a fake OAuth identity
provider, real MCP server processes, a recorded Qwen tool call, and the service files checked
against a fake service manager. The design decisions are in `docs/adr/0001`–`0008`.

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
