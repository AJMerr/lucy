# Reminders and scheduled tasks

Ask Lucy to remind you of something, or to do something on a schedule, in any chat: the
terminal, the window or the sprite.

- **Reminders** just tell you something at a time: "remind me at 5 to call Mom", "remind me
  every Monday at 9 to send the timesheet". No model runs, so they're instant and free.
- **Tasks** are things she does on her own at a time: "every weekday at 8, summarise my email
  and calendar", "on the 1st of each month, list my subscriptions from my email". She runs the
  task as an agent and reports back.

Times are your local time. Schedules can be once, daily, on weekdays, on chosen days of the
week, monthly, or every so many minutes.

## You always see it first

Before anything is scheduled she shows an approval card with exactly what and when, for
example:

```
Task “Morning summary” · weekdays at 08:00 · level 1 · profile local-qwen
Summarise my email and calendar for today
```

Allow it, or deny it and tell her what to change. ("Always" doesn't skip this: every new
reminder or task asks.)

## Where results go

Reminders and task results land in a chat called **Tasks**, and you get a notification:

- With Lucy's app running, a desktop notification, and the sprite says it in her bubble (after
  she finishes what she's saying, if she's talking).
- Without the app, the core sends the desktop notification itself.

## What a task may do

A task runs at the agent level you choose when it's made (the card shows it; change it in the
Tasks list):

| Level | While you're away |
| --- | --- |
| 0 · supervised | Nothing that would ask runs, so effectively nothing |
| 1 · guarded (default) | Reads (files in your home folder, and MCP servers' data like your email and calendar) |
| 2 · workspace | Also changes files and runs commands in your workspace |
| 3 · unrestricted | Anything |

Nobody is there to approve anything, so a call the level doesn't allow is **refused on the
spot**: she carries on with what she can and says in the result what she couldn't do. Raise
the task's level if you want it to do more. Agent mode's step and cost caps (Settings → Chat &
agent) apply to every run, so a task can't run away with your API credit.

A task uses the model of the chat you made it in; change it in the Tasks list.

## Seeing and changing them

- **The window:** ◷ Tasks (bottom of the chat list) lists everything with when it runs next.
  Pause or resume, reword what it does, change a task's level or model, or delete it.
- **The terminal:** `lucy tasks` lists them; `lucy tasks pause|resume|delete <name>` (the
  title, part of it, or the ID).
- **In a chat:** "what have I got scheduled?", "cancel the morning summary".

## Lucy needs to be running

Reminders and tasks run in Lucy's core, so they only happen while it's running. Started on
demand (by `lucy` or the app), the core stays up while anything is scheduled, but it won't be
running after a reboot until something starts it. To have it always running, turn on **Run at
login** in Settings → Service & startup, or:

```sh
lucy service install
```

She offers this when you schedule your first task, and the Tasks list says when it's off.

A run missed while the core wasn't running (the computer was off, say) happens **once** when
it starts again, not once per missed day; then the schedule carries on as normal.
