# 3. IPC: local socket by default, optional HTTP/WebSocket

Date: 2026-11-05 · Status: Accepted

## Decision

- **Default:** JSON-RPC over a local socket (Unix domain socket on Linux/macOS, named pipe on
  Windows). No network port; access is controlled by file permissions.
- **Optional:** HTTP + WebSocket listener for remote/browser clients. Off by default. When
  enabled it binds `127.0.0.1` and requires a token. LAN/Tailscale binding is a separate,
  explicit opt-in.
- Message types live in `lucy-protocol` and are shared by every side.
- The Tauri sprite's Rust side connects to the socket and relays events to its webview.

## Consequences

- Two transports to keep in sync; both must route through the same request handler.
- Protocol needs a version field so old front ends fail clearly against a newer core.

## Implementation notes (v0.1.0)

- Socket: `$LUCY_SOCKET`, else `$XDG_RUNTIME_DIR/lucy/core.sock` (folder mode 0700), else a
  per-user temp folder; Windows uses the named pipe `lucy-core-<user>`. A stale socket file left
  by a crashed core is removed on startup; a second core finding a live one exits quietly.
- `lucy core run` is the core; any front end spawns it detached when nothing answers, logging to
  `<data dir>/core.log`. It exits after 30 idle minutes (`LUCY_CORE_IDLE_SECS`), where idle means
  no connections and no reply streaming.
- Methods: `hello` (version check), `subscribe`, `chat.send`/`chat.cancel`,
  `sessions.list/load/rename/delete/resolve`, `session.settings/set_profile/set_plain`,
  `profiles.list`. Replies stream as `event` notifications (`user_message`, `turn_started`,
  `delta`, `turn_finished`) to the client that sent the message and to subscribers.
- Per-session `/model` and `/plain` choices live in the core's memory, not the database.
- `lucy-client` holds the client side so the sprite can reuse it.
- The Windows named pipe allows only the user running the core (and SYSTEM), the counterpart
  of the 0700 socket folder on Unix (v0.3.0).

## Implementation notes (v0.2.0, protocol v4)

- `core.status` (version, PID, `service`, busy chats) and `core.shutdown {force}` work before
  `hello` and keep their shape across protocol versions, so a newer client can always see and
  stop an older core. Shutdown refuses while replies are streaming unless forced; forced, it
  cancels them (partial replies are kept, as with `chat.cancel`), stops accepting, waits up
  to 2 s for replies and clients to wind down, then closes MCP servers and exits.
- A client whose `hello` is refused by an older core asks for its status and, if it's idle,
  shuts it down and starts its own (`lucy` prints that it did). A busy or newer core is left
  alone with a message naming `lucy core stop`. Cores before v4 still need `pkill` once.

## Implementation notes (v0.3.0, remote access)

- `[remote] enabled = true` starts a WebSocket listener at `ws://<bind>/rpc` (axum) next to
  the local socket. Each text frame is one JSON-RPC message, handled by the same code as the
  socket (`serve_client`), so the two transports can't drift. The config is read when the core
  starts.
- **Token:** keyring entry `remote-token` (`LUCY_KEY_REMOTE_TOKEN` overrides it), created by
  `lucy remote token` and checked on every connection, so `--rotate` applies to new
  connections at once. It's sent as `Authorization: Bearer`, or by browsers (which can't set
  WebSocket headers) as a `lucy-token.<token>` subprotocol offered next to `lucy`. Compared in
  constant time. With no token set, connections get 503.
- **Network:** a bind beyond loopback also needs `allow_network = true`, and logs a warning:
  there's no TLS. Use Tailscale or an SSH tunnel, or a TLS proxy in front (clients take
  `wss://`). A LAN or Tailscale bind is therefore still the explicit opt-in this ADR asked for.
- **What remote clients get:** everything a local client does, except that their chats run in
  `[agent] workspace` (else the core user's home) rather than the folder they send, at most at
  `[remote] max_level` (default 1), and they can't `core.shutdown` (they couldn't start the
  core again). `hello` reports the folder (`folder`, only for remote clients).
- **Client side:** `Endpoint::Remote { url, token }` in `lucy-client`; both transports feed
  the same reader and writer tasks. The CLI picks the remote core from `--remote`, then
  `LUCY_REMOTE`, then `remote.toml` (next to `config.toml`, written by `lucy remote connect`),
  with the token in the keyring entry `remote-client` (`LUCY_KEY_REMOTE_CLIENT`). It never
  spawns a remote core. `lucy service` and `lucy core stop` always act on the local core.

## Implementation notes (v0.5.0, protocol v5: settings over RPC)

- For the GUI (and a phone app later), settings go through the core instead of each front end
  editing files: `config.get` returns config.toml as JSON with defaults filled in, its path,
  and whether home-manager owns it; `config.update {patch}` applies a JSON merge patch to the
  file with `toml_edit`, so comments and layout survive, writes only a valid result, and says
  when remote-access changes need a restart. The core already re-reads config.toml per
  request, so other changes apply to the next message.
- `keys.status` lists every key name the config refers to, where it's set (keyring or an
  environment variable) and what uses it; `keys.set` and `keys.remove` write the keyring of
  the core's machine. Nothing ever returns a key's value.
- `persona.get|set|reset` edit your own version of the configured persona
  (`personas/<name>.md`); reset deletes it, back to the built-in one.
- `mcp.login {name}` runs the OAuth sign-in in the core, opening the browser on its machine,
  and answers when it's done. Adding and removing MCP servers is a `config.update`;
  `tools.list` reports any that fail to start.
- Remote clients can read settings and key status. Changing settings, keys or the persona
  needs `[remote] allow_config = true`; MCP sign-in is refused remotely regardless. Service
  actions (install, restart) stay out of the protocol: the desktop app runs `lucy service`.

## Implementation notes (v1.1.0, protocol v6: remote access locked down)

- **What a remote client may call** is an allowlist (`server/remote.rs`, `authorize`): anything
  not named is refused. Settings, keys, the persona, memory, scheduled tasks and `models.*` need
  `[remote] allow_config`; `core.shutdown`, `mcp.login` and `remote.rotate` are never allowed.
  Session methods that change a chat (`chat.clear`, `sessions.delete`, `session.set_*`, undo …)
  only work on chats started from another machine (a `remote` column on sessions); a level set
  remotely is clamped to `max_level` when stored. Other clients' incognito chats are hidden from
  lists, events, `core.status` and lookups. `config.get` hides MCP `env` and `headers` values
  without `allow_config`.
- **Approvals carry where they came from.** A pending approval records whether its reply is
  remote and which connection started it. A remote client may answer only its own replies' calls,
  and only `deny` unless `[remote] approvals = "remote"`; the `tool_approval` event says
  `local_only` so clients can show it. "Always" from or in a remote reply counts once. Memory
  changes in a remote reply ask; tasks scheduled from one are marked `remote` and run at most at
  `max_level` (re-checked when they fire).
- **Keys** given by name to `models.list`/`models.try` only go to an address a profile or preset
  pairs with that name. Lucy's own keyring entries (`remote-*`, `mcp-oauth-*`) are never used as
  API keys, by profiles, MCP servers or `keys.set`/`keys.remove`.
- **TLS:** by default (`[remote] tls`) the core serves `wss://` with a self-signed certificate it
  makes once and keeps in the keyring (`remote-tls`). Clients pin its SHA-256 fingerprint
  (`Endpoint::Remote { pin }`, `remote.toml` `fingerprint`); a proxy's real certificate is checked
  with webpki roots when no pin is set. Plain `ws://` is only sent to loopback or Tailscale
  addresses (judged by the connected peer) unless `insecure`. Each core gets its own client
  token entry (`remote-client-<host>-<port>`).
- **The listener** has its own accept loop: at most 32 connections, a 10-second deadline for
  TLS and the request headers, 32 MiB messages, and an `Origin` check against
  `[remote] allowed_origins` (Lucy's own clients send none). The token is cached in memory and
  must be at least 32 characters. `remote.rotate` replaces it and closes every remote
  connection; a `config.update` that changes `[remote]` restarts the listener straight away, and
  turning it off closes connections.
- **The local socket** is only trusted in a folder of the user's own that others can't write to
  (Unix); otherwise another account could stand in for the core in a shared `/tmp`.
