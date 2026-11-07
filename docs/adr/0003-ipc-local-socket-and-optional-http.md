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

## Implementation notes (milestone 1, step 6)

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
  of the 0700 socket folder on Unix (step 11).

## Implementation notes (step 10, protocol v4)

- `core.status` (version, PID, `service`, busy chats) and `core.shutdown {force}` work before
  `hello` and keep their shape across protocol versions, so a newer client can always see and
  stop an older core. Shutdown refuses while replies are streaming unless forced; forced, it
  cancels them (partial replies are kept, as with `chat.cancel`), stops accepting, waits up
  to 2 s for replies and clients to wind down, then closes MCP servers and exits.
- A client whose `hello` is refused by an older core asks for its status and, if it's idle,
  shuts it down and starts its own (`lucy` prints that it did). A busy or newer core is left
  alone with a message naming `lucy core stop`. Cores before v4 still need `pkill` once.

## Implementation notes (step 11, remote access)

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
