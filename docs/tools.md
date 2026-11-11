# Tools and agent mode

Lucy can do things, not just talk: read, list and search files, fetch web pages, run commands,
and write or edit files. [MCP servers](mcp.md) add more (email, calendars, GitHub, a browser…).

## Asking first

In an ordinary chat (**Assist** mode), she decides what to use, and you decide what runs:

- **Just happens:** reading, listing and searching files in your home folder, and tools an MCP
  server marks read-only.
- **Asks first:** anything that changes files, runs a command or goes online. You see what she
  wants to do (and, for file edits, the changes) with **Allow**, **Always** (this tool, for the
  rest of this chat) and **Deny**; in a terminal, `y`, `a` or `n`.
- **Always asks:** private paths like `~/.ssh`, keys and `.env` files, whatever the mode.

`/tools` in the terminal lists every tool and whether it runs by itself or asks. `/last` shows
the last tool call's full input and output.

## Agent mode

In agent mode she works a task through on her own: she plans, uses tools, checks her work and
reports back. Switch a chat with the **Agent** button in its header or `/mode agent`, or give
her a task straight from the terminal:

```sh
lucy agent "add tests for parser.rs"
lucy agent -l 2 "fix the build"        # at level 2
```

The **level** decides what she may do without asking:

| Level | Runs by itself | Still asks |
| --- | --- | --- |
| 0 Supervised | nothing | every tool call |
| 1 Guarded (default) | reads; file changes inside the folder you started in | commands, going online, anything outside the folder |
| 2 Workspace | anything local inside `[agent] workspace` (or the folder), commands included | going online, anything outside the workspace |
| 3 Unrestricted | everything | nothing |

Private paths ask at every level. Commands that run without asking at level 2 can still do
anything a shell can, including installing software or downloading, so pick that level for
workspaces you'd let a script loose in.

**Caps:** every task stops after `[agent] max_steps` model steps, and at `max_cost_usd` when the
profile has prices (`price_in`, `price_out`; local models count as free) or `max_tokens` when it
doesn't. At a cap she gets one last turn to sum up what's done and what's left; say "continue"
to give her a fresh budget. `lucy agent` exits with code 2 when that happens.

Chats remember their mode, level and model across restarts.

## Undo

Every reply's file changes (from `write_file` and `edit_file`) are recorded. **Undo** (in the
window's header, or `/undo`) lists the last reply's changes and, if you confirm, restores them,
deleting files that reply created. Run it again to go further back (up to 20 replies per chat).
Changes made by `run_command` or MCP servers' tools can't be undone this way.

## Rules

Settings → Chat & agent, or `[tools]` in config.toml:

```toml
[tools]
builtin = true          # Lucy's own tools; false leaves only MCP servers' tools
max_rounds = 10         # tool rounds per message before she must answer

[tools.rules]           # override the usual rule: "auto", "ask" or "deny"
run_command = "deny"
github = "ask"          # every tool from the `github` MCP server
```

`private_paths` replaces the built-in list of paths that always ask. Add `tools = false` to a
profile whose model can't call tools.

## For scripts

```sh
lucy ask "what's a flake?"      # one answer, plain text, then exit
git diff | lucy ask "review"    # the prompt can come from stdin
lucy ask -y "tidy todo.md"      # approve every tool call (only for scripts you trust)
```

`lucy ask` prints the answer on stdout and tool activity on stderr. When a tool needs approval it
asks on your terminal, even if the question was piped in. With no terminal (cron, CI) it
declines, unless you pass `--yes`.

## Lucy's tools in other programs

`lucy mcp serve` works the other way round: it offers Lucy's built-in tools to any MCP client
over stdio, working in the current folder.
