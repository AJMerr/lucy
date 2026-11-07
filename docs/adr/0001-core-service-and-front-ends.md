# 1. Core service with separate front ends

Date: 2026-11-05 · Status: Accepted

## Context

Lucy needs a terminal client now and a desktop sprite later, and the sprite must be fully
toggleable: off means no screen space and no resources. Lucy must run on Linux, macOS and Windows.

## Decision

- A long-running **core** (`lucy-core`) owns all state: provider profiles, sessions, persona,
  MCP tools, mode/permission enforcement.
- **Front ends** (`lucy` CLI, the Tauri sprite, future remote clients) are thin processes that
  talk to the core. Toggling the sprite = starting/stopping its process.
- The core **starts on demand**: any front end spawns it if it isn't running. `lucy service
  install` optionally registers it as a login service (systemd user unit / launchd / Windows
  startup).
- Written in **Rust**. TypeScript is used only where it is vastly easier (the sprite's webview UI).

## Consequences

- One source of truth for history and settings across all front ends.
- Need an IPC protocol and versioning discipline (see ADR 0003).
- Front ends must handle "core not running / starting up" gracefully.

## Implementation notes (step 10)

- `lucy core run --service` runs until stopped (no idle exit). The login service starts it:
  a systemd user unit (`lucy-core.service`, `WantedBy=default.target`, `Restart=on-failure`)
  on Linux, a launchd agent (`io.github.ajmerr.lucy.core`, `RunAtLoad`, `KeepAlive` on crash)
  on macOS, and an `HKCU\…\Run` value on Windows. No admin rights anywhere.
- The unit and plist get the installing shell's `PATH` and `LUCY_*` settings (never
  `LUCY_KEY_*`), and pin `LUCY_SOCKET` to the socket that shell's `lucy` uses. That pin is
  also how a front end knows the service is "its" core: it starts the core through the
  service manager only when the service's socket matches its own, so test or `LUCY_SOCKET`
  cores never wake the real service. Windows has nothing to pin (the `Run` value can't set
  variables), so it uses the service only for the default pipe.
- Unix service managers send the core's output to `core.log`; on Windows the core detaches
  from its console and opens the log itself.
- Stopping is graceful (`core.shutdown`, ADR 0003): a clean exit leaves the service stopped
  until the next start, which goes through the service manager again.
