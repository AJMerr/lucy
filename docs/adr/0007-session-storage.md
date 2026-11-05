# 7. Sessions in SQLite, identified by ULIDs

Date: 2026-11-05 · Status: Accepted

## Decision

- Conversation history lives in one SQLite database owned by the core:
  `$LUCY_DATA_DIR/lucy.sqlite`, else `<platform data dir>/lucy/lucy.sqlite`.
  `rusqlite` with the `bundled` feature, so no system SQLite is needed on any OS.
- Sessions are identified by **ULIDs** (26 chars, time-sortable, globally unique) so they could
  later sync across machines. Front ends accept any unique, case-insensitive ID prefix.
- Each message stores role and content, plus for assistant replies the **profile and model** that
  produced it, an **incomplete** flag for replies cut off mid-stream, and optional **token
  usage**.
- A session's title defaults to its first user message (first line, max 60 chars incl. `…`);
  rename overrides it. LLM-written titles may come later.
- The list is ordered by a monotonic activity counter rather than timestamps, so ordering is
  exact even within one millisecond.
- Schema changes are append-only migrations tracked with `PRAGMA user_version`.

## Consequences

- Only the core process should open the database (ADR 0001); front ends go through IPC.
- Long IDs need prefix matching and tab-completion in the CLI.
