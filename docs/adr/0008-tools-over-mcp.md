# 8. Tools over MCP, with built-ins and core-enforced approvals

Date: 2026-11-06 · Status: Accepted (built in steps 8a, 8b and 8c)

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

**External servers (8b).** `[mcp.servers.<name>]` gives a command, arguments, plain `env`,
`env_keys` (variable → keyring entry, resolved like model keys, so `LUCY_KEY_*` overrides),
an optional `cwd` (default home) and `enabled`. The core's `McpHost` starts each server the
first time a chat needs tools (all of them in parallel, 60 s to finish the handshake), shares
it across chats, and restarts it on next use if it died or its config changed. Servers
removed from the config are stopped, and all are stopped when the core exits. A server that
won't start is logged and listed by `tools.list` (`/tools`, `lucy mcp list`); the chat goes on
without it. Commands are found on `PATH` the way a shell would (`npx.cmd` on Windows), and
servers' stderr goes to the core log.

- Tools are always exposed as `<server>__<tool>` (sanitised to `[A-Za-z0-9_-]`, at most 64
  characters), so names never change when another server is added. Built-ins keep short names.
- `[tools.rules]` keys match a tool's exposed name first, then its server's name.
- Relative paths in a server's read-only calls are checked against that server's folder.
- `lucy mcp add` takes flags and asks for anything missing (name, command, then keyring
  variables). It edits config.toml with `toml_edit`, keeping comments, refuses edits that
  wouldn't load, then starts the server once and lists its tools. `lucy mcp serve` offers the
  built-ins to other MCP clients over stdio.

**Remote servers (8c).** `url = "https://…"` instead of `command`; rmcp's streamable HTTP
client does the talking. Credentials, in order:

- `bearer_key` (keyring entry, sent as `Authorization: Bearer …`), `header_keys` (header →
  keyring entry) and plain `headers`. A 401 then names the entries to fix with `lucy auth set`.
- Otherwise OAuth, through rmcp's `OAuthState`: discovery (protected-resource and
  authorization-server metadata), client registration (dynamic, unless `oauth_client_id` names
  a pre-registered app), PKCE, code exchange and refresh. `lucy mcp login <name>` binds a
  localhost port (`oauth_redirect_port`, or any free one), opens the browser and waits up to
  5 minutes for the redirect. Tokens and the client registration are stored as JSON in the
  keyring entry `mcp-oauth-<name>` (a custom rmcp `CredentialStore`), and refreshed by rmcp's
  `AuthClient` when they expire or the server rejects them. A refused refresh, or no sign-in
  at all, gives "run `lucy mcp login <name>`". `lucy mcp logout` deletes the entry.
- `lucy mcp add --url` checks the server and, if it wants a sign-in, offers to log in.

Remote tools follow their labels like any other, but read-only ones skip the home-folder
path checks: their paths name the remote service's data, not this machine's files.

## Consequences

- Built-ins are code we maintain and secure. `run_command` always asks unless overridden.
- MCP labels are hints from the server's author. A server that labels a destructive tool
  read-only would run without asking, so only add servers you trust, or set `ask` rules.
- "Always" approvals live in memory and reset when the core restarts.
- Agent mode (step 9) will reuse the same verdicts, with `AgentLevel` deciding which asks
  become runs.
