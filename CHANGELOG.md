# Changelog

## v1.0.1: security fixes (2026-11-13)

Fixes for problems found in a security review of tool approvals and the terminal. Please
upgrade.

### Breaking for security

- **Always and `auto` no longer skip the safety checks.** Private paths (`~/.ssh`, keys,
  `.env` files and so on) ask even for a tool you approved with **Always** or set to `auto`
  in `[tools.rules]`, and so do file changes outside the folder Lucy may write in (the
  workspace at level 2). Level 3 still changes files anywhere but private paths. See
  [docs/tools.md](docs/tools.md).

### Fixed

- **Paths that climb out of the working folder are caught.** A path like `new/../../.bashrc`
  counted as inside the working folder while `new` didn't exist yet, so in agent mode a file
  change could land outside it, private paths included, without asking. Paths are now judged
  the way the system resolves them, including links to things that don't exist yet.
- **`search_files` stays out of private folders** reached another way, such as `~/./.config`
  or a link to one.
- **The terminal shows control characters instead of obeying them.** Replies, tool names,
  arguments, output and errors could carry terminal escapes or a carriage return, which could
  paint a harmless command over the real one in an approval prompt, write to the clipboard or
  hide where a link goes. They're now shown as symbols (`␛`, `␍`, `<U+202E>`), and a command
  in an approval prompt also shows characters that only look like spaces. `lucy ask` does
  this on a terminal and passes replies through unchanged when piped. Picture data that isn't
  base64 is no longer drawn.
- **Signing in to an MCP server only opens web pages:** the sign-in address must be `https:`
  (or `http:` on this computer). On Windows it opens without `cmd`, which read `&` in the
  address as the start of another command and cut ordinary sign-in addresses short.
- [docs/stability.md](docs/stability.md) says what each part of a version number means and
  how security fixes ship.

### Upgrading from v1.0.0

- Restart the core after installing: `lucy service restart`, or `lucy core stop` if you
  don't use the service. Still protocol v5.
- If a tool you set to `auto` or approved with Always now asks, it's touching a private path or
  a file outside the folder: the change above.

## v1.0.0: Lucy goes public (2026-11-12)

The first stable release: Lucy is open source, documented, installable from package managers,
and has her own look.

- **Open source** under MIT OR Apache-2.0, with CONTRIBUTING, SECURITY (private vulnerability
  reporting), a code of conduct, and issue and pull request templates.
- **A docs site** at [ajmerr.github.io/lucy](https://ajmerr.github.io/lucy/): new guides for
  installing (including getting past the unsigned-installer prompts on macOS and Windows), the
  first five minutes, the app and sprite, the terminal and config, tools and agent mode, the
  login service, remote access, Nix and troubleshooting. CI checks every link.
- **A README for newcomers**, pointing into the guides.
- **Install from Homebrew** (`brew install --cask ajmerr/lucy/lucy`, or the `lucy` formula) and
  **the AUR** (`lucy-bin`), updated by each release.
- **What stays stable:** from 1.0, config.toml and the `lucy` command follow semver; see
  [docs/stability.md](docs/stability.md).
- **Her own look:** the sprite now ships with an original kawaii netrunner (the built-in
  `default` pack, now the default): she blinks, talks, thinks with a thought bubble, and looks
  happy, frustrated or attentive. The plain `placeholder` pack is still built in to copy
  from. A pack you made yourself is unchanged if your config names it.
- **ComfyUI and Ollama share the GPU:** before each picture, Lucy unloads the models of the
  chat's Ollama on this computer, so ComfyUI has the whole card; the chat model loads again for
  her reply. Turn it off with Settings → Pictures → Free the GPU first
  (`[images.comfyui] unload_ollama`). See [docs/images.md](docs/images.md).
- **Settings → Voice:** "Read typed replies aloud" and "Hey Lucy" are greyed out until Talk to
  Lucy is on, since neither works without it.

### Upgrading from v0.10.0

- Restart the core after installing: `lucy service restart`, or `lucy core stop` if you
  don't use the service. Still protocol v5.
- The sprite takes on her new look unless your config names a pack (`[sprite] pack`); to keep
  the old one, set `pack = "placeholder"`.

## v0.10.0: polish (2026-11-11)

The rough edges a newcomer hits first. See [docs/models.md](docs/models.md) and
[docs/mcp.md](docs/mcp.md).

- **Add a model without knowing its ID:** Settings → Models & keys → Add a model…, or
  `lucy config add-model`. Pick a provider, paste its key, choose from the models it actually
  offers (searchable), try it with a test message, and save it with its context size and
  whether it can see pictures filled in. OpenAI is now one of the providers on offer.
- **Long chats fit:** Lucy learns each model's context size (or takes `context_tokens` from the
  profile). Near it, older turns are replaced by a summary she writes once and keeps with the
  chat; recent turns stay word for word, and the chat still shows everything.
- **`/clear` and `/compress`** in the terminal, the window and the sprite: start her afresh,
  or summarise the chat so far now. The window marks both in the chat.
- **`/model <name> --default`** and **Make default** in a chat's header also make that model
  the default for new chats and the sprite. The sprite shows her model in her input box.
- **MCP made easy:** ready-made Google Workspace, GitHub and web browser servers in Settings →
  MCP servers, and pasting a server's command line (split into the program, its arguments and
  variables). docs/mcp.md walks through making a Google OAuth client.
- **An old core is spotted:** `lucy` and the app say when the running core is older than them,
  and the app can restart it.
- Protocol: `chat.clear`, `chat.compress`, `models.list` and `models.try`; saved messages
  can be markers (`marker`); `session.set_profile` takes `make_default`. Still protocol v5.

## v0.9.0: voice (2026-11-11)

Talk to Lucy and hear her, in her desktop app. See [docs/voice.md](docs/voice.md).

- **Push-to-talk:** hold the 🎙 in the main window's message box or by the sprite's input box
  (or click it and talk until you pause), or bind `lucy sprite talk` on Wayland. What you say
  goes to that chat as a message.
- **She answers aloud,** a sentence at a time as the reply streams in, leaving code blocks,
  tables and link addresses in the chat. Talking again or Stop cuts her off. **Read typed
  replies aloud** does the same for what you type.
- **Her voice:** Kokoro or Piper on your computer, or OpenAI or ElevenLabs (with a key in the
  keyring). **Hearing:** Whisper tiny, base, small or turbo, on your computer.
- **"Hey Lucy":** an optional wake word that starts the sprite listening.
- **Settings → Voice** sets all of this, previews her voice, and gets voice ready: the speech
  engine (sherpa-onnx) and models download into the data folder the first time, with progress.
  Nothing is bundled, and the engine is loaded at run time, so building Lucy needs none of it.
- `[voice]` in config.toml; `lucy sprite talk`.

## v0.8.0: scheduled tasks (2026-11-10)

Lucy can remind you of things and do things on a schedule. See [docs/tasks.md](docs/tasks.md).

- **Reminders** ("remind me at 5 to call Mom") show their message at the time; no model runs.
- **Tasks** ("every weekday at 8, summarise my email and calendar") are prompts she runs on her
  own as an agent, at the level you choose (0-3), with agent mode's step and cost caps. Calls
  the level doesn't allow are refused on the spot and reported, since nobody's there to ask.
- **Always shown first:** her `schedule` tool shows an approval card with what, when, the level
  and the model before anything is saved. She can also list and cancel schedules.
- **Results** land in a **Tasks** chat with a notification: from Lucy's app (a desktop
  notification and the sprite's bubble) or, without the app, from the core itself.
- **Managing them:** the window's ◷ Tasks list (pause, resume, reword, level, model, delete) and
  `lucy tasks [pause|resume|delete <name>]`.
- **Running:** an on-demand core stays up while anything is scheduled; she offers the login
  service when you schedule your first task. A run missed while the core was stopped happens
  once when it starts.
- **Protocol** (still v5, additions only): `schedules.list|update|delete`, the `notification`
  event, and `subscribe {notifications}` for clients that show notifications themselves.

## v0.7.0: memory (2026-11-10)

Lucy remembers you. See [docs/memory.md](docs/memory.md).

- **She saves facts about you on her own** (preferences, people, projects) in `memory.md`, with
  a short note when she does, and every chat starts with them. Sensitive facts (health, money,
  relationships, addresses, ID numbers) ask first; anything that looks like a secret is refused.
  "Forget that" removes one, and she replaces facts that change.
- **Big memories stay fast:** past about 8 KB, chats get your core facts plus the relevant ones,
  picked by keyword, or by meaning with an embedding model (`[memory] embedding_profile`), and
  she can search the rest with `recall`.
- **Settings → Memory** lists everything with edit and forget, or edit it as text; `lucy
  memory` and `lucy memory edit` in the terminal. `[memory] enabled = false` turns it off.
- **Incognito chats:** `lucy --incognito`, `/new incognito`, or New incognito chat in the window.
  No memory, and deleted (pictures too) when you leave.
- **Protocol** (still v5, additions only): `memory.get`, `memory.set`, `chat.send.incognito`,
  `SessionSummary.incognito`.

## v0.6.0: pictures (2026-11-10)

Lucy can see pictures and make them. See [docs/images.md](docs/images.md).

### Seeing

- **Send her pictures:** paste, drop or attach them in the main window, or drop one on the
  sprite. Models marked **Can see pictures** (`vision = true`) get the picture; for others, a
  **vision helper** (`[images] vision_profile`) describes it once, and the description is kept.
- **She can look at picture files:** `read_file` shows PNG, JPEG, GIF and WebP files as
  pictures, and pictures from any MCP tool reach the model instead of being dropped.
- Pictures are kept with the chat in your data folder; `images.get` fetches one.

### Making

- **Ask her for a picture** and she writes the prompt and calls `generate_image`; or type
  **`/image <description>`** in the terminal, the window or the sprite to make one straight
  away.
- **`[images] generator`:** an OpenRouter image model (asks before each picture, since it
  costs money) or **ComfyUI** on your own GPU (free, runs by itself), with Lucy's built-in
  text-to-image workflow or your own exported workflow using `{{prompt}}`, `{{negative}}`,
  `{{width}}`, `{{height}}`, `{{seed}}` and `{{checkpoint}}`.
- Pictures are saved to `~/Pictures/Lucy` (`[images] folder`) and shown in the main window,
  in the sprite's bubble, and inline in Ghostty, kitty and WezTerm.
- **Settings → Pictures** sets all of this; a ComfyUI setup guide for NixOS is in
  docs/images.md.

### Protocol

- Still v5; additions only: `chat.send` takes `images`, `images.get` and `images.generate`,
  and `user_message` and `tool_finished` events carry the pictures that came with them.

## v0.5.0: Lucy's app (2026-11-08)

Lucy now has an app: one program, "Lucy", with her sprite, a main window for your chats and
every setting, and a tray icon. Anyone can install it and set her up without a terminal, and
the core can now be configured by any front end, ready for a phone app later.

### Settings over the core

- **Protocol v5:** the core can now show and change Lucy's settings for any front end:
  config.toml (`config.get`, `config.update`, keeping your comments and layout and refusing
  anything invalid), API keys (`keys.status|set|remove`, never sending a key back), your
  persona (`persona.get|set|reset`), and MCP sign-in (`mcp.login`).
- **`[remote] allow_config`** (off by default): other machines can read settings but only
  change them, or keys and the persona, when this is on.
- A new `lucy` replaces an older idle core by itself; restart a service core with
  `lucy service restart`.

### Lucy's main window

- **`lucy app`** opens Lucy's main window: your chats in a sidebar (new, rename, delete, a dot
  where something's new), the open chat with streaming replies, tool approvals, Stop and Undo,
  and the chat's model, Assist/Agent mode, level and plain mode in its header. Replies in
  other chats, and chats started from a terminal or the sprite, show up as they happen. On a
  narrow window the chat list folds into a menu.
- **A tray icon** opens the window, shows or hides the sprite, or quits. Closing the window
  keeps Lucy in the tray. (Linux needs AppIndicator for the tray; `lucy app` works without.)
- "more…" on the sprite's bubble and `lucy sprite chat` now open her chat in the main window,
  which replaces the sprite's separate chat window.
- The same page works in a browser against a core with remote access on
  (`app.html?url=ws://host:7077/rpc&token=…`), ready for a phone app later.

### Settings in the app

- **Settings** (bottom of the chat list): models and API keys, how new chats start, agent caps
  and tool rules, your name, the persona, MCP servers (with sign-in for remote ones), the
  sprite, remote access (and its token), and the login service. Each section has Save and
  Revert, sends only what changed, and shows the core's reason if it refuses a change. Keys go
  to the keyring and are never shown back. A config owned by home-manager can be looked at but
  not changed here.
- **First-run setup:** with no config yet, the window starts by asking for a provider, model,
  key and your name, with the same choices as `lucy config init` (both now come from the core,
  `setup.presets`).

### One app: "Lucy"

- **The sprite, the main window and the tray are one app, "Lucy"** (the `lucy-desktop`
  program, replacing `lucy-sprite`). Opened from your app menu it shows the window; `lucy app`
  and `lucy sprite …` work as before. Installers are named `Lucy_…`.
- **`[sprite] enabled`** (Settings → Sprite): off, the app is just its window and tray when it
  starts, and `lucy sprite` still shows her.
- **`[app] autostart`** starts the app at login, in the background (the sprite if she's
  enabled, and the tray). `[sprite] autostart` from v0.4.0 still works, and
  `lucy service install` replaces its old `lucy-sprite` login entry.
- **Nix:** `.#lucy-sprite` is now `.#lucy-desktop`, and home-manager's `programs.lucy.sprite.*`
  is now `programs.lucy.desktop.*` (the old names still work, with a warning). Its unit is
  `lucy-desktop.service`.
- The desktop app logs to `desktop.log` (next to `core.log`).

### Upgrading from v0.4.0

- Install the new `lucy` and the app (an installer, Nix, or from source; the program is now
  `lucy-desktop`), then restart the core: `lucy service restart`, or `lucy core stop` if you
  don't use the service. The protocol is now v5; a new `lucy` replaces an older idle core by
  itself, but a service core needs the restart.
- Built from source? Remove the old `lucy-sprite` from wherever you copied it (e.g.
  `~/.cargo/bin`); `lucy sprite` now looks for `lucy-desktop`.
- If the sprite started at login, run `lucy service install` once: it swaps the old
  `lucy-sprite` login entry for the app's.
- home-manager: `programs.lucy.sprite.*` still works but warns; rename it to
  `programs.lucy.desktop.*`. The unit is now `lucy-desktop.service`.

### Known limitations

- **The tray icon needs AppIndicator on Linux** (`libayatana-appindicator`, and GNOME's
  AppIndicator extension). Without it the app skips the tray; `lucy app` opens the window.
  Builds made with plain `cargo` in the Nix dev shell can't find it on NixOS; the Nix package
  can.
- **Installers still aren't signed**, so macOS and Windows warn on first run.
- **No mobile app yet.** The main window works in a browser against a core with remote access
  on (`app.html?url=ws://host:7077/rpc&token=…`).

## v0.4.0: desktop sprite (2026-11-07)

Lucy can now live on your desktop: an animated sprite in the corner of the screen who answers
in speech bubbles, asks before running tools, and opens her whole chat in a window when a reply
runs long. She uses the same core as `lucy`, so her chat carries on in the terminal and
vice versa. There are installers for Linux, macOS and Windows, and a Nix flake with a
home-manager module.

### Desktop sprite

- **Lucy on your desktop.** `lucy sprite` starts her: a small transparent figure in the
  bottom-right corner, above your windows, animating from her sprite pack.
  `lucy sprite quit` sends her off.
- **No window rules needed** on Wayland compositors with layer shell (Hyprland, Sway, KDE and
  others); elsewhere she's an always-on-top window.
- **Sprite packs:** `[sprite] pack` and `scale` now work. Packs live in
  `~/.local/share/lucy/packs/`, with `placeholder` built in. A broken pack is explained in
  `sprite.log` and replaced by the placeholder.
- **She talks.** Click her (or press your hotkey) and type; her reply streams into a speech
  bubble with light Markdown, capped at about 8 lines with "more…", fading after
  `bubble_timeout_secs`. She listens, thinks, talks, cheers or frowns as the reply goes.
- **Approvals in the bubble:** when a tool needs your go-ahead she asks, with Allow, Always
  and Deny (and the diff for edits).
- **Her own chat** ("Sprite" in `/sessions`) carries on across restarts, with whichever core
  `lucy` uses (local or remote).
- **`lucy sprite ask`** opens her input box and **`lucy sprite toggle`** hides or shows her;
  bind them in your compositor on Wayland. Elsewhere `[sprite] hotkey` (default
  `Ctrl+Shift+Space`) works by itself.
- Only her visible parts take clicks on layer-shell compositors; the space around her passes
  clicks through.
- **Her chat window.** "more…" on a long reply, or `lucy sprite chat`, opens her whole chat in
  an ordinary window: streaming replies, the tools she ran, approvals, an input box and Stop.
  While it's open, replies go there instead of the bubble.
- **At login:** `[sprite] autostart = true` makes `lucy service install` start her at login
  too (an XDG autostart entry, a launch agent, or a `Run` value); `uninstall` takes her off.

### Installing

- **Installers:** publishing a release attaches `.deb`, `.rpm`, `.AppImage`, `.dmg` (Apple
  silicon) and `.msi` builds of the sprite, each with `lucy` inside. They're unsigned for now.
- **Nix flake:** `.#lucy`, `.#lucy-sprite` (Linux) and `.#default`, plus an overlay.
- **home-manager module:** `programs.lucy` installs Lucy and her sprite, can write
  `config.toml` (`settings`), and runs the core (`service.enable`) and the sprite
  (`sprite.autostart`, with your graphical session) as systemd user services. `lucy service
  status|restart` and `lucy core stop` treat its core unit as theirs.
- When home-manager owns `config.toml`, `lucy config init` and `lucy mcp add|remove` say so
  instead of failing on a read-only file.

### Upgrading from v0.3.0

- Install with `cargo install --path crates/lucy-cli` (or an installer, or Nix), then restart
  the core: `lucy service restart`, or `lucy core stop` if you don't use the service. The
  protocol is still v4, so the new `lucy` won't replace the old core by itself.
- To try the sprite from source, build `apps/sprite` (see its README) and put `lucy-sprite`
  next to `lucy`. The new `[sprite] hotkey` and `autostart` settings have defaults; nothing
  in your config needs changing.
- Moving to the home-manager module's `service.enable`? Run `lucy service uninstall` first, so
  the two don't both define `lucy-core.service`.

### Known limitations

- **Placeholder art only.** No Lucy character art ships; make a pack from `packs/placeholder`.
- **Installers aren't signed**, so macOS and Windows warn on first run, and the macOS build is
  for Apple silicon only. Installing one doesn't put `lucy` on your `PATH` on macOS or Windows.
- **Global hotkeys don't work on Wayland**; bind `lucy sprite ask` in your compositor.
- **GNOME on Wayland** has no layer shell, so she's an ordinary floating window there.

## v0.3.0: remote access (2026-11-07)

Lucy's core can now serve other machines: your laptop's `lucy` can use your desktop's core,
with its chats, models, keys and MCP servers, over WebSocket. It's off by default, behind a
token, and capped so a leaked token can't run commands unapproved.

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
- The v0.2.0 limitations still apply (Windows console flash at login, level 2 commands,
  `/undo` scope, Ollama context), and there's no desktop sprite or voice yet.

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

- **Windows login:** a console window may flash briefly when the service starts the core.
- **Level 2 runs commands without asking.** A command can do anything a shell can, including
  installing software or downloading, so use level 2 only in workspaces you'd let a script
  loose in.
- **`/undo` covers `write_file` and `edit_file` only**, not changes made through `run_command`
  or other servers' tools.
- **Short Ollama context.** Ollama's default 4,096-token window fills faster with tools. Raise
  `OLLAMA_CONTEXT_LENGTH` (e.g. 16384) if replies lose track.
- **No desktop sprite or voice yet**, and no HTTP/remote access to the core.

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
- A model: a local server such as Ollama, or an API key.
- On Linux, a Secret Service provider (e.g. gnome-keyring) if you want keys in the keyring.

### Known limitations

- **No tools yet.** Lucy can't read files, run commands or browse, and `/mode agent` is a
  placeholder.
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
