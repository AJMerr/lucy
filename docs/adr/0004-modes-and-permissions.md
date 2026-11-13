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

## Implementation notes (v1.0.1, what rules can't skip)

- Config rules and "always" approvals no longer come before everything. Private paths, and
  file changes outside the area writes may run in (the chat's folder, or the workspace at level
  2), are checked first and ask whatever the rule or approval says; level 3 still writes
  anywhere but private paths. `deny` and `ask` rules apply first as before.
- Paths are judged the way the system will resolve them: `..` after a folder that doesn't
  exist yet counts from where that folder will be, and a link to something that doesn't exist
  yet is judged by its target. Before, `new/../../.bashrc` counted as inside the working folder.
- `search_files` walks the real path of its folder, and matches private paths against both
  that and the path it was given, so `~/./.config` or a link to a private folder is still
  private.

## Implementation notes (v1.1.0, home and protected files)

- **Home isn't a write area.** The folder (or workspace) writes may run in is never home or a
  folder holding it, since chats with no folder of their own work in home; there, changes ask,
  and so do commands at level 2 when the workspace is home.
- **Protected files** (`DEFAULT_PROTECTED_PATHS`: shell startup files, autostart, systemd user
  units, `~/.local/bin`, git hooks and config, `.envrc`, editor tasks) and Lucy's own files
  (the config, `remote.toml`, personas, chats, memory, pictures, voice models and sprite packs,
  each named, so a project beside them isn't caught) ask before any change, at every level and whatever the rules; from
  another machine they're refused.
- **Private paths** cover developer, cloud and AI tool credentials, browser and mail profiles,
  shell histories, Lucy's own chats and more key file types; `extra_private_paths` adds to the
  list.
- **fetch_url** reaches public addresses through a resolver and redirect policy that refuse
  private ones; an address on this machine or a private network asks every time, and only
  follows redirects on that host. Pages are read up to 5 MB.
- **Approved calls are checked again** before they run; one that would now be refused, or ask
  for a different reason, doesn't. Undo leaves a file that has since become a link alone.
- Commands and MCP servers don't inherit `LUCY_KEY_*` or the providers' own key variables.
