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
