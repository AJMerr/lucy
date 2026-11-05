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
- Open: the Windows named pipe uses the default security descriptor; restrict it to the current
  user before the HTTP/remote work (step 11).
