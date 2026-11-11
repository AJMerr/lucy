# Lucy

A model-agnostic personal AI assistant with a netrunner's attitude. Talk to her in the terminal,
or let her live on your desktop as an animated sprite with speech bubbles that you can toggle on
and off.

> **Status:** v0.9.0: terminal chat with any OpenAI-compatible or Anthropic model, able to
> read, search, run and edit with your approval, use any MCP server you add, and work through
> tasks on her own within limits you set. She remembers you across chats, reminds you of things
> and runs scheduled tasks, talks and listens in her app, looks at pictures you show her and
> makes pictures with OpenRouter image models or ComfyUI. The core can run from login and serve
> your other machines. Lucy's app puts her on the desktop as an animated sprite, with a main
> window for your chats and every setting, and a tray icon; no terminal needed. Installers, a Nix
> flake and a home-manager module. See the [release notes](CHANGELOG.md).

## What works today

- **Any model.** OpenAI-compatible APIs (Ollama, OpenAI, Qwen/DashScope, OpenRouter, LM Studio,
  llama.cpp, vLLM…) and Anthropic, set up as named profiles. Add one without knowing its ID:
  pick the provider, paste its key, choose from the models it offers and try it (Settings →
  Models & keys, or `lucy config add-model`). Switch per chat with `/model`. See
  [docs/models.md](docs/models.md).
- **Long chats fit.** Near a model's context size, older turns are summarised and recent ones
  kept word for word; the chat itself keeps everything. `/clear` starts her afresh and
  `/compress` summarises now, in the terminal, the window and the sprite.
- **Lucy's voice, facts intact.** She stays in character, but accuracy, clean code and plain safety
  warnings come first. `/plain` drops the character entirely. The persona is a Markdown file you
  can override.
- **Streaming chat with Markdown.** Headings, lists, inline code and syntax-highlighted code
  blocks render as the reply arrives. Ctrl+C stops a reply and keeps what was said.
- **Tools, with your say-so.** Lucy can read, list and search files, fetch web pages, run
  commands, and write or edit files. Reads in your home folder just happen; anything that
  changes files, runs a command or goes online asks first (`y`, `a` for always this chat, `n`).
  Private paths like `~/.ssh` and `.env` files always ask.
- **Any MCP server.** Add tool servers in Settings → MCP servers (ready-made Google Workspace,
  GitHub and web browser entries, or paste a server's command) or with `lucy mcp add`. Lucy
  starts them when needed, fills their tokens from your keyring, and applies the same approval
  rules. See [docs/mcp.md](docs/mcp.md).
- **She remembers you.** Lucy saves facts about you as you chat (your preferences, the people
  in your life, your projects) and every chat starts with them. She notes when she saves
  something, asks first for anything sensitive, and never keeps secrets. See and change it all
  in Settings → Memory or with `lucy memory`; incognito chats (`lucy --incognito`) remember
  nothing and aren't kept. See [docs/memory.md](docs/memory.md).
- **Reminders and scheduled tasks.** "Remind me at 5 to call Mom", or "every weekday at 8,
  summarise my email and calendar": she shows you what and when first, then reminds you, or does
  the task on her own at the level you chose and reports back in a Tasks chat, with a desktop
  notification. Manage them in the window's Tasks list or with `lucy tasks`. See
  [docs/tasks.md](docs/tasks.md).
- **Talk to her.** In Lucy's app, hold the mic (or bind `lucy sprite talk`, or say "Hey Lucy")
  and talk: Whisper hears you on your computer, and she answers aloud as her reply streams in,
  in a local voice (Kokoro, Piper) or a hosted one (OpenAI, ElevenLabs). See
  [docs/voice.md](docs/voice.md).
- **Pictures.** Show her a picture (paste, drop or attach it in the window, drop it on the
  sprite, or have her open a picture file) and models that can see look at it; for those that
  can't, a vision helper describes it. Ask her to draw something, or type `/image <description>`,
  and she makes it with an OpenRouter image model or ComfyUI on your own GPU. Pictures are saved
  to `~/Pictures/Lucy` and shown in the window, the sprite's bubble, and inline in Ghostty,
  kitty and WezTerm. See [docs/images.md](docs/images.md).
- **Agent mode.** `/mode agent` or `lucy agent "task"`: Lucy works through a task step by step
  on her own, at a freedom level you choose (0–3), within step and cost caps. `/undo` rolls back
  the files a reply changed.
- **Saved conversations.** Every chat is stored locally in SQLite. Continue the last one with
  `lucy -c`, or resume any by ID prefix. Incognito chats are the exception: deleted when you
  leave them.
- **Keys kept out of files.** API keys live in your OS keyring, with environment-variable
  overrides.
- **One background core.** The `lucy` command starts it on demand and it exits after 30 idle
  minutes, or `lucy service install` keeps it running from login. Future front ends (like the
  sprite) share it.

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

Other ways to install:

- **Installers** for Lucy's desktop app, with `lucy` inside: `.deb`, `.rpm` and `.AppImage`
  for Linux, `.dmg` for macOS (Apple silicon) and `.msi` for Windows, attached to each
  [release](https://github.com/AJMerr/lucy/releases). They aren't signed yet, so macOS and
  Windows warn the first time (on macOS, right-click the app and choose Open). On Linux `lucy`
  lands in `/usr/bin`. On macOS it's inside the app (`Lucy.app/Contents/MacOS/lucy`) and on
  Windows next to `lucy-desktop.exe` in the install folder; put it on your `PATH` to use it in
  a terminal.
- **Nix:** `nix profile install github:AJMerr/lucy` installs both (`.#lucy` is just the CLI;
  `.#lucy-desktop`, the desktop app, is Linux-only), or use the overlay. With
  [home-manager](https://github.com/nix-community/home-manager):

  ```nix
  # flake inputs: lucy.url = "github:AJMerr/lucy";
  imports = [ lucy.homeModules.default ];
  programs.lucy = {
    enable = true;
    service.enable = true;            # the core from login (instead of `lucy service install`)
    desktop = { enable = true; autostart = true; };  # the app, with your graphical session
    settings = {                      # optional: config.toml, read-only once set
      defaults.profile = "local-qwen";
      profiles.local-qwen = {
        provider = "openai_compat";
        base_url = "http://localhost:11434/v1";
        model = "qwen3:14b";
      };
    };
  };
  ```

  Leave `settings` out to keep editing `config.toml` yourself (or with `lucy config init` and
  `lucy mcp add`); once home-manager owns it, those commands say so instead of changing it.
  The units are systemd user services; the app's starts with `graphical-session.target`,
  which uwsm, GNOME and KDE sessions reach.

**Local models:** install [Ollama](https://ollama.com), then run `ollama pull qwen3:14b` and pick
"Local Qwen via Ollama" in the wizard. On NixOS, set `services.ollama.enable = true;`, and use
`package = pkgs.ollama-cuda;` for NVIDIA GPUs.

## Using Lucy

```sh
lucy                       # new chat
lucy -c                    # continue your most recent chat
lucy -r 01m6ne             # resume a chat by any unique ID prefix
lucy --incognito           # a chat with no memory, deleted when you leave it
lucy memory                # what Lucy remembers about you (`lucy memory edit` to change it)
lucy tasks                 # reminders and scheduled tasks (`lucy tasks pause|resume|delete <name>`)
lucy ask "what's a flake?" # one answer, plain text, then exit
lucy ask -y "tidy todo.md" # approve every tool call (only for scripts you trust)
git diff | lucy ask        # the prompt can come from stdin
lucy agent "add tests for parser.rs"   # a task Lucy works through on her own (agent mode)
lucy agent -l 2 "fix the build"        # …at level 2: anything local in the workspace runs
```

In the chat:

| Command | What it does |
| --- | --- |
| `/model [profile] [--default]` | list profiles, switch this chat to one, or also make it the default |
| `/clear` | she starts afresh from here (the chat above stays; also in the window and the sprite) |
| `/compress` | summarise the chat so far, to free up her context (also in the window and the sprite) |
| `/image <description>` | make a picture straight away (also in the window and the sprite) |
| `/plain [on\|off]` | drop Lucy's voice (toggles without an argument) |
| `/new [incognito]`, `/sessions` | start a new chat (incognito: no memory, not kept), list saved chats |
| `/resume <id>`, `/rename <title>`, `/delete <id>` | manage saved chats |
| `/tools` | list tools and whether each runs by itself or asks |
| `/last` | show the last tool call's full input and output |
| `/mode [assist\|agent]` | show or switch this chat's mode |
| `/level [0-3]` | how freely Lucy works in agent mode (see below) |
| `/undo` | restore the files the last reply changed; repeat to go further back |
| `/help`, `/quit` | the list of commands, leave |

Keys: Tab completes commands and profile names, Alt+Enter (or a trailing `\`) starts a new line,
Ctrl+C stops a reply, and Ctrl+D quits. Start a message with `//` to send a literal `/`.

Other commands: `lucy config path|add-model`, `lucy auth set|remove|status`, `lucy memory [edit]`,
`lucy tasks`, `lucy mcp add|list|remove|login|logout`,
`lucy service install|uninstall|status|restart`, `lucy core stop`,
`lucy remote connect|disconnect|token`, `lucy sprite [ask|talk|chat|toggle|quit]`, `lucy app`.

`lucy ask` prints the answer on stdout and tool activity on stderr. When a tool needs approval
it asks on your terminal, even if the question was piped in. With no terminal (cron, CI) it
declines, unless you pass `--yes`.

### Agent mode

In agent mode Lucy works a task through on her own: she plans, uses tools, checks her work,
and reports back. The level decides what she may do without asking you:

| Level | Runs by itself | Still asks |
| --- | --- | --- |
| 0 Supervised | nothing | every tool call |
| 1 Guarded (default) | reads; file changes inside the folder you started in | commands, going online, anything outside the folder |
| 2 Workspace | anything local inside `[agent] workspace` (or the folder), commands included | going online, anything outside the workspace |
| 3 Unrestricted | everything | — |

Private paths (`~/.ssh`, keys, `.env` files) ask at every level. Commands run without asking at
level 2 can still do anything a shell can, including installing software or downloading, so
pick that level for workspaces you'd let a script loose in.

Every task has caps: `[agent] max_steps` model steps, and `max_cost_usd` when the profile has
`price_in`/`price_out` (local models count as free) or `max_tokens` when it doesn't. At a cap,
Lucy gets one last turn to sum up what's done and what's left; say "continue" to give her a
fresh budget. `lucy agent` exits with code 2 when that happens. Chats remember their mode,
level and model across restarts.

Every reply's file changes (from `write_file` and `edit_file`) are recorded. `/undo` lists the
last reply's changes and, if you confirm, restores them, deleting files that reply created.
Run it again to go further back (up to 20 replies per chat). Changes made through
`run_command` or other servers' tools can't be undone this way.

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

### MCP servers

The app's Settings → MCP servers has ready-made Google Workspace, GitHub and web browser
entries, and takes a pasted command. [docs/mcp.md](docs/mcp.md) covers those, including making
the Google OAuth client. In a terminal:

```sh
lucy auth set github                     # store the token in the keyring
lucy mcp add github --key GITHUB_PERSONAL_ACCESS_TOKEN=github \
  -- npx -y @modelcontextprotocol/server-github
lucy mcp add                             # or answer a few questions instead
lucy mcp list                            # which servers start, and how many tools each has
lucy mcp remove github

# a remote server: offers to sign in (browser, OAuth); `lucy mcp login linear` does it later
lucy mcp add linear --url https://mcp.linear.app/mcp
# or one that takes an API key from the keyring
lucy mcp add acme --url https://mcp.acme.dev/mcp --bearer-key acme
```

A server's tools show up as `<server>__<tool>` (e.g. `github__create_issue`). Servers start
the first time a chat needs tools and keep running until the core stops; one that crashes is
restarted on next use. Tools the server labels read-only run by themselves (with the same
path checks as the built-ins); everything else asks. Set `github = "ask"` (or `"deny"`) under
`[tools.rules]` to override a whole server. `lucy mcp add` keeps your config's comments and
layout.

Remote servers are reached over streamable HTTP. Sign-in uses OAuth: Lucy registers herself
with the server, opens your browser, catches the redirect on localhost, and keeps the tokens in
your keyring, refreshing them when they expire. If a refresh is refused, `/tools` and `lucy mcp
list` say to run `lucy mcp login` again; `lucy mcp logout` forgets a sign-in. Servers that use
API keys take `--bearer-key` or `--header-key HEADER=keyname` instead. A remote server's
read-only tools run by themselves; its file paths aren't on your machine, so the home-folder
checks don't apply.

`lucy mcp serve` works the other way round: it offers Lucy's built-in tools to any MCP client
over stdio, working in the current folder.

### Running the core at login

The `lucy` command starts Lucy's core in the background when it's needed, and the core exits
after 30 idle minutes (it stays up while reminders or tasks are scheduled). To keep it running
from login instead, so scheduled tasks run after a reboot too, install it as a service:

```sh
lucy service install      # start now and at every login
lucy service status       # installed? running how, which version, busy with what?
lucy service restart      # after upgrading Lucy, to run the new build
lucy service uninstall    # back to starting on demand
lucy core stop            # stop whichever core is running (asks if Lucy is mid-reply; -f cancels)
```

It's a systemd user unit on Linux (`~/.config/systemd/user/lucy-core.service`), a launchd
agent on macOS (`~/Library/LaunchAgents/io.github.ajmerr.lucy.core.plist`), and a value under
`HKCU\…\CurrentVersion\Run` on Windows. None of them needs admin rights. The service runs with
the `PATH` and `LUCY_*` settings of the shell you installed it from (so `run_command` and MCP
servers find the same programs); re-run `install` if those change. API keys aren't copied
into it, so keep them in the keyring. On Windows the service sees your user environment
variables instead, and a console window may flash briefly at login.

When a newer `lucy` finds an older core running, it restarts the core by itself as long as
the core isn't in the middle of a reply.

To have Lucy's desktop app start at login too, set `autostart = true` under `[app]` in
`config.toml` (or tick it in Settings → Sprite) and run `lucy service install` again. That
adds the app to your login items, starting in the background (the sprite, if she's enabled, and
the tray): an autostart entry in `~/.config/autostart/` on Linux, a launch agent on macOS, a
second `Run` value on Windows. Setting it back to `false` (and installing again) or
`lucy service uninstall` takes it off. (`[sprite] autostart` from v0.4.0 still works, and its
old `lucy-sprite` login entry is replaced.)

### Using Lucy from another machine

Your laptop's `lucy` can use your desktop's core: the same chats, models, keys and MCP
servers, with tools running on the desktop. On the desktop, turn remote access on in
`config.toml` and restart the core:

```toml
[remote]
enabled = true
bind = "100.64.0.7:7077"    # your Tailscale address (or 127.0.0.1 behind an SSH tunnel)
allow_network = true        # needed for anything beyond 127.0.0.1
```

```sh
lucy service restart        # or `lucy core stop`
lucy remote token           # prints the token the laptop needs
```

On the laptop (which needs no config or keys of its own):

```sh
lucy remote connect desktop:7077   # asks for the token; checks it works
lucy                               # now chats with the desktop's core
lucy remote disconnect             # back to the laptop's own core
lucy --remote desktop:7077 ask "…" # or just once (LUCY_REMOTE works too)
```

Things to know:

- **No TLS.** Traffic is plain WebSocket, so use a network you trust: Tailscale encrypts it
  for you, or use an SSH tunnel, or put a TLS proxy in front and connect with `wss://`.
  Binding beyond 127.0.0.1 needs `allow_network = true`.
- **Tools run on the core's machine**, in `[agent] workspace` if you set one, else its home
  folder; the chat banner says which. The laptop's current folder isn't used.
- **Remote chats are capped at agent level 1** (`[remote] max_level`), so a leaked token
  can't run commands without someone approving them.
- `lucy remote token --rotate` replaces the token. A remote client can't stop the core.
- The core only listens while it runs, so keep it running with `lucy service install`.

To change Lucy's voice, copy [`persona/lucy.md`](persona/lucy.md) to
`~/.config/lucy/personas/lucy.md` and edit it. Text between `<!-- voice -->` markers is dropped in
plain mode.

### Where things live

| What | Default (Linux) | Override |
| --- | --- | --- |
| Config | `~/.config/lucy/config.toml` | `LUCY_CONFIG` |
| Chats, history, core log | `~/.local/share/lucy/` | `LUCY_DATA_DIR` |
| Pictures she makes | `~/Pictures/Lucy/` | `[images] folder` |
| What she remembers | `~/.local/share/lucy/memory.md` | `LUCY_DATA_DIR` |
| Core socket | `$XDG_RUNTIME_DIR/lucy/core.sock` | `LUCY_SOCKET` |

### Troubleshooting

- **Something's wrong with the core:** check `~/.local/share/lucy/core.log`.
- **You upgraded or rebuilt Lucy but the old behaviour sticks:** the previous core is still
  running. `lucy` and the app say so when it's older than them (the app's banner has a Restart
  button). Run `lucy service restart` (or `lucy core stop` if you don't use the service), and
  the next `lucy` starts a fresh one. Cores from before this release don't know `core stop`: use
  `pkill -f "lucy core run"` once.
- **The service won't start:** `lucy service status`, then the log above. On Linux,
  `systemctl --user status lucy-core` and `journalctl --user -u lucy-core` show what systemd
  saw.
- **Keyring errors on Linux:** you need a Secret Service provider (e.g. gnome-keyring), or set
  `LUCY_KEY_<NAME>` instead.
- **"config.toml is managed by home-manager":** `programs.lucy.settings` owns it; change Lucy's
  settings in your Nix config, or set it to `null` to edit the file yourself again.

## How it's built

```
crates/
  lucy-protocol/   JSON-RPC messages and events shared by every side
  lucy-providers/  the Provider trait + openai_compat and anthropic streaming adapters
  lucy-core/       the background core: config, keys, sessions, personas, tools, MCP servers
  lucy-client/     connects front ends to the core (and starts it on demand)
  lucy-cli/        the `lucy` command: chat loop, Markdown renderer, setup wizard
  lucy-tools/      built-in tools (files, search, fetch, shell) as an in-process MCP server
  lucy-packs/      sprite packs: loading and checking pack.toml and its frames
apps/desktop/      the desktop app's Rust side (Tauri; its own Cargo workspace)
apps/ui/           its web pages (the sprite and the main window), shared with a later mobile app
persona/           built-in personas
packs/placeholder/ the only sprite pack tracked in git
nix/               the flake's packages, home-manager module and its check
docs/              ADRs, guides, example config
```

The design decisions are recorded in [`docs/adr/`](docs/adr/).

## Development

```sh
cargo test --workspace                                # everything runs offline
cargo clippy --workspace --all-targets -- -D warnings
nix develop                                           # optional; has what the Tauri sprite needs
nix flake check                                       # builds the packages, checks the home-manager module
```

Tests run against fake model servers and recorded real streams. To record a new stream:
`cargo run -p lucy-providers --example capture`. To preview the system prompt Lucy would send:
`cargo run -q -p lucy-core --example system_prompt`. CI runs fmt, clippy and the tests on Linux,
macOS and Windows, builds the sprite on all three, and runs `nix flake check`. Publishing a
GitHub release builds the installers and attaches them (`.github/workflows/release.yml`; run
it by hand with no tag to try a build).

## Desktop app

Lucy's desktop app ("Lucy", the `lucy-desktop` program) is her sprite, her main window and a
tray icon in one. Install it from a release or the Nix flake (above), or build it from
[`apps/desktop`](apps/desktop/README.md); `[app] autostart` starts it at login.

The sprite puts Lucy on your desktop: a small animated figure in the bottom-right corner,
above your windows (on Wayland compositors with layer shell, like Hyprland, Sway or KDE, no
window rules needed). Click her, or bind `lucy sprite ask` to a key, and type: she answers in
speech bubbles, and asks there before running tools. With voice on, talk to her instead (the 🎙
by her input box, `lucy sprite talk`, or "Hey Lucy") and she answers aloud. `lucy sprite toggle`
hides her and `lucy sprite quit` closes the app. Don't want her on screen? Turn her off in
Settings → Sprite (`[sprite] enabled = false`): the app is then just its window and tray, and
`lucy sprite` still shows her when you want her.

`lucy app` (or the tray icon, or "more…" on a long reply) opens Lucy's main window: your chats
on the left, the open chat on the right, with streaming replies, tool approvals, Stop, Undo,
and the chat's model, mode, level and plain mode in its header. Closing it leaves her in the
tray. The tray needs AppIndicator on Linux (`libayatana-appindicator`; GNOME also needs its
AppIndicator extension); without it, `lucy app` still opens the window.

**Settings** (bottom of the chat list) cover everything in `config.toml` and more, without a
terminal: models and their API keys (sent to the keyring, never shown again), how new chats
start, agent caps and tool rules, your persona, what she remembers, her voice, MCP servers (and
signing in to remote ones), pictures (what makes them, where they're saved, which model
describes them), the sprite, remote access and its token, and the login service. Each section
saves on its own, and the core refuses anything that would break the config, saying why. With no
config yet, the window starts with first-run setup instead (the same providers as
`lucy config init`).

No character art is committed. Lucy loads sprite packs from your data directory
(`~/.local/share/lucy/packs/` on Linux). Copy `packs/placeholder`, swap the frames, and point
`[sprite] pack` at it. See [ADR 0005](docs/adr/0005-sprite-tauri-and-packs.md).

## Licence

All rights reserved for now. A licence will be chosen before the repo goes public.
Lucy is a fan-inspired name; this project is not affiliated with CD Projekt or Studio Trigger.
