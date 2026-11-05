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
