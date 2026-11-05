# 4. Assist and Agent modes, with configurable agent freedom

Date: 2026-11-05 · Status: Accepted

## Decision

Tools are provided via **MCP** servers. Two modes:

- **Assist** — chat plus tools. Read-only tools run automatically; anything risky (writes,
  shell, outbound) asks for confirmation every time.
- **Agent** — autonomous multi-step tasks, governed by an `AgentLevel`:

| Level | Name         | Behaviour                                                         |
|-------|--------------|-------------------------------------------------------------------|
| 0     | Supervised   | Confirm every tool call                                           |
| 1     | Guarded      | **Default.** Run freely; confirm irreversible or outbound actions |
| 2     | Workspace    | Run freely inside a configured directory; confirm outside it      |
| 3     | Unrestricted | Never confirm                                                     |

Every agent task has a **step cap** and a **token/cost cap** regardless of level.

## Consequences

- Each tool needs a risk classification (read-only / local-write / irreversible / outbound).
  MCP tool annotations are used where present; otherwise a local override table.
- Permission enforcement lives in the core, never in front ends.
