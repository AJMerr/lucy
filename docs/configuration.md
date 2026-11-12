# The terminal and config.toml

## The `lucy` command

```sh
lucy                       # new chat
lucy -c                    # continue your most recent chat
lucy -r 01m6ne             # resume a chat by any unique ID prefix
lucy --incognito           # a chat with no memory, deleted when you leave it
lucy ask "what's a flake?" # one answer, then exit (see Tools for scripts)
lucy agent "task"          # a task she works through on her own
```

Setting up: `lucy config init` (first model), `lucy config add-model`, `lucy config path`,
`lucy auth set|remove|status`. Everything else: `lucy memory [edit]`, `lucy tasks`,
`lucy mcp add|list|remove|login|logout|serve`, `lucy service install|uninstall|status|restart`,
`lucy core stop`, `lucy remote connect|disconnect|token`,
`lucy sprite [ask|talk|chat|toggle|quit]`, `lucy app`. Each takes `--help`.

## In a chat

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

`/clear`, `/compress` and `/image` also work in the app's window and the sprite.

## config.toml

`lucy config init` or the app's setup writes it, in `~/.config/lucy/config.toml` on Linux
(`lucy config path` prints where). The app's Settings edit it for you, keeping your comments and
layout, and so does `lucy mcp add`. [config.example.toml](config.example.toml) documents every
setting. The core picks up most changes straight away; the few that need a restart (like remote
access) say so.

## Keys

Keys never go in config.toml, which only names them (`key_name = "anthropic"`):

- Settings → Models & keys, or `lucy auth set <name>`, stores a key in the OS keyring (Secret
  Service, Keychain or Credential Manager). It's never shown again.
- `LUCY_KEY_<NAME>` overrides it, and vendor variables like `ANTHROPIC_API_KEY` are used as a
  fallback.
- `lucy auth status` shows where each key comes from, without printing it.

## Her character

Lucy's persona is a Markdown file. Edit it in Settings → Persona, or copy
[`persona/lucy.md`](https://github.com/AJMerr/lucy/blob/main/persona/lucy.md) to
`~/.config/lucy/personas/lucy.md` and change it there. Text between `<!-- voice -->` markers is
dropped in plain mode (`/plain`). Accuracy, clean code and plain safety warnings come before the
character, whatever the file says.

## Where things live

| What | Linux | macOS | Windows | Override |
| --- | --- | --- | --- | --- |
| Config | `~/.config/lucy/` | `~/Library/Application Support/lucy/` | `%APPDATA%\lucy\` | `LUCY_CONFIG` (the file) |
| Chats, memory, voice models, core log | `~/.local/share/lucy/` | `~/Library/Application Support/lucy/` | `%APPDATA%\lucy\` | `LUCY_DATA_DIR` |
| Pictures she makes | `~/Pictures/Lucy/` | `~/Pictures/Lucy/` | `Pictures\Lucy` | `[images] folder` |
| Core socket | `$XDG_RUNTIME_DIR/lucy/core.sock` | a temporary folder | a named pipe | `LUCY_SOCKET` |
