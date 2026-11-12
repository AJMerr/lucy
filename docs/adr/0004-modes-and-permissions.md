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

## Implementation notes (v0.2.0, agent mode)

- **Entry points.** `/mode agent` per chat, `/level 0-3`, and `lucy agent "task" [--level N]
  [--yes]` (exit code 2 at a cap). Agent mode uses `defaults.agent_profile` unless the chat
  picked a profile. A chat's profile, plain flag, mode and level are saved in the session
  database (schema v3); "always" approvals stay in memory.
- **Verdicts** are a pure function (`lucy_core::policy::decide`) over the call's kind (read,
  write, command, online, other), whether it's remote, and the paths it names:

  | | Assist | 0 | 1 Guarded | 2 Workspace | 3 |
  |---|---|---|---|---|---|
  | Read | in home | ask | in home or folder | in home or workspace | run |
  | Write | ask | ask | in the folder | in the workspace | run |
  | Command | ask | ask | ask | if the folder is in the workspace | run |
  | Online / unknown | ask | ask | ask | ask | run |
  | Remote read | run | ask | run | run | run |

  Private paths ask at every level, symlinks resolved. The workspace defaults to the chat's
  folder. Config rules and "always" approvals apply first.
- **Caps.** Steps count model responses. Cost uses the providers' own token counts
  (`stream_options.include_usage` for OpenAI-compatible servers; `message_start`/`message_delta`
  for Anthropic, cache tokens counted as input) and per-profile prices; unpriced local models
  are free, and other unpriced models use `max_tokens`. At a cap, the model gets one more step
  with an instruction to summarise and no more tool calls (tools stay declared, since some
  providers require it once tools are in the history; stray calls are refused). The turn ends
  `capped` with its usage.
- **Undo.** Before a built-in `write_file`/`edit_file` runs, the core stores the file's previous
  contents (or "didn't exist") per reply, last 20 replies per chat, files up to 10 MB.
  `session.undo` lists and restores the newest reply's set. Command and external-tool changes
  aren't tracked.
