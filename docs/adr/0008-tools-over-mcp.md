# 8. Tools over MCP, with built-ins and core-enforced approvals

Date: 2026-11-06 · Status: Accepted (8a built; 8b and 8c planned)

## Context

ADR 0004 says tools come from MCP servers and risky calls need confirmation in Assist mode.
This records how: which MCP client, which tools Lucy ships, how risk is judged, and how the
user approves.

## Decision

**MCP client: `rmcp`**, the official Rust SDK. We write only the glue.

**Built-in tools, served over MCP.** Lucy ships `read_file`, `list_dir`, `search_files`,
`fetch_url`, `run_command`, `write_file` and `edit_file` (crate `lucy-tools`). They run as an
in-process MCP server, one per chat turn, rooted at the folder the user is working in
(`chat.send` carries `cwd`). Because they go through the same client as external servers,
they get the same rules and approvals. `[tools] builtin = false` turns them off.

**Risk comes from MCP tool labels, plus path rules and overrides.**

| Tool | Assist mode |
|---|---|
| `readOnlyHint` and not `openWorldHint` | Runs by itself if every path it touches is inside home and not private; otherwise asks |
| Anything else, including unlabelled tools (the spec's defaults are "destructive, open world") | Asks |
| `[tools.rules] name = "auto" \| "ask" \| "deny"` | Overrides both |

Private paths default to credentials and keys (`~/.ssh`, `~/.gnupg`, cloud CLIs, keyrings,
`.env*`, `*.pem`, `*.key`, SSH key names). Symlinks are resolved before checking. Path-like
arguments are `path`, `paths`, `file`, `dir`, `directory`, `root` and `*_path`/`*_dir`; a
tool with an optional `path` is checked against the working folder. `search_files` never
reads private, hidden or git-ignored files.

**Approvals are enforced in the core**, never in front ends:

- The core emits `tool_approval` (call, reason, optional diff preview) and waits. Any client
  may answer with `tool.approve`: `once`, `always` (this tool, rest of the session, in memory)
  or `deny`. The first answer counts. With no answer within 5 minutes, the call is declined.
- A declined call becomes an error result telling the model not to retry unless asked.
- Every call is announced with `tool_started` and `tool_finished` (full output), so front ends
  can show compact lines and a `/last` view.
- `lucy ask` prompts on the controlling terminal (`/dev/tty`, `CONIN$`) even when stdin is
  piped. With no terminal it declines. `--yes` approves everything.

**Loop limits.** At most `[tools] max_rounds` (default 10) rounds of tool calls per message.
After that, calls get a "limit reached, answer now" result. One more round fails the turn.
Tool output is capped at 100 KB.

**Persistence.** Tool calls and results are saved as messages (schema v2: `tool_calls`,
`tool_call_id`, `is_error`). A reply interrupted between asking for tools and getting results
leaves calls unanswered, which providers reject. So when history is loaded, missing results
are filled with an "interrupted" error.

**Protocol v2.** New events and methods, and `cwd` on `chat.send`. A v1 client talking to a v2
core (or the reverse) gets the usual version error. Restart the core after upgrading.

**Providers.** Both adapters send tool definitions and tool history in their vendor's format,
and assemble streamed tool calls (OpenAI fragments by index; Anthropic `input_json_delta`) into
whole `ToolCall` events. Garbled argument JSON is passed on as a string so the core can tell
the model. Profiles can opt out with `tools = false`.

## Planned

- **8b:** external MCP servers started as child processes, under `[mcp.servers.<name>]`, plus
  `lucy mcp add|list|remove`. Server tools are prefixed if names collide.
- **8c:** remote MCP servers over streamable HTTP, with an API key from the keyring, or OAuth
  via `lucy mcp login <server>`. That opens a browser, catches the redirect on a temporary
  localhost port, and keeps the tokens in the keyring, refreshing them automatically.

## Consequences

- Built-ins are code we maintain and secure. `run_command` always asks unless overridden.
- MCP labels are hints from the server's author. A server that labels a destructive tool
  read-only would run without asking, so only add servers you trust, or set `ask` rules.
- "Always" approvals live in memory and reset when the core restarts.
- Agent mode (step 9) will reuse the same verdicts, with `AgentLevel` deciding which asks
  become runs.
