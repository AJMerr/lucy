# The app and the sprite

Lucy's app (the `lucy-desktop` program) is three things in one: her **sprite** on your desktop,
her **main window** with your chats and every setting, and a **tray icon**. It needs no
terminal, and starts Lucy's core itself when it isn't running.

## The sprite

A small animated figure in the bottom-right corner of your screen, above your windows. On
Wayland compositors with layer shell (Hyprland, Sway, KDE…) she needs no window rules.

- **Click her** (or bind `lucy sprite ask` to a key) and type. She answers in a speech bubble,
  and asks there before running tools. Drop a picture on her to send it with your next message.
- **Talk to her** with the 🎙 by her input box, `lucy sprite talk`, or "Hey Lucy", once voice
  is on. See [Voice](voice.md).
- **"more…"** on a long reply opens the whole chat in the main window.
- Her input box shows which model she's on. `/clear`, `/compress` and `/image` work there too.
- Reminders and finished tasks show in her bubble.
- `lucy sprite toggle` hides her, `lucy sprite quit` closes the app.

She has one long-running chat of her own, titled "Sprite", which you can also open in the
window or with `lucy -r`. Don't want her on screen? Turn her off in Settings → Sprite
(`[sprite] enabled = false`): the app is then just its window and tray, and `lucy sprite` shows
her when you want her.

## The main window

`lucy app`, the tray icon, or "more…" opens it: your chats on the left, the open chat on the
right, with streaming replies, tool approvals, Stop and Undo. The chat's header switches its
model (with **Make default**), its mode (Assist or Agent), its agent level and plain mode.
Paste, drop or attach pictures in the message box. Closing the window leaves her in the tray.

**Settings** (at the bottom of the chat list) cover everything in config.toml and more:

| Page | What it sets |
| --- | --- |
| Models & keys | your models, adding one, the default, how new chats start, and API keys (sent to the keyring, never shown again) |
| Chat & agent | agent caps, tool rules, your name |
| Persona | her character, as Markdown |
| Memory | what she remembers about you |
| Voice | hearing, her voice, the wake word, and getting voice ready |
| MCP servers | tool servers, ready-made ones, and signing in to remote ones |
| Pictures | what makes them, where they're saved, which model describes them |
| Sprite | on or off, her pack and size, how long replies stay up, a hotkey, starting with your login |
| Remote access | serving your other machines, and the token |
| Service & startup | running the core from login, and restarting it |

Each page saves on its own, and the core refuses anything that would break your config, saying
why. **Tasks**, in the chat list, shows your reminders and scheduled tasks.

## The tray

The tray icon opens the window, shows or hides the sprite, and quits. On Linux it needs
AppIndicator (`libayatana-appindicator`; GNOME also needs its AppIndicator extension); without
it, `lucy app` still opens the window.

## Starting with your login

Tick it in Settings → Sprite (or set `[app] autostart = true` and run `lucy service install`).
The app then starts in the background at login: the sprite, if she's on, and the tray.

## Sprite packs

How she looks comes from a **pack**: a folder with a `pack.toml` and its frames, one set per
state (idle, thinking, talking, and optionally happy, error and listening). To make your own,
copy [`packs/placeholder`](https://github.com/AJMerr/lucy/tree/main/packs/placeholder) to
your data folder's `packs/` (`~/.local/share/lucy/packs/<name>/` on Linux), swap the frames,
and pick it in Settings → Sprite (`[sprite] pack = "<name>"`). See
[ADR 0005](adr/0005-sprite-tauri-and-packs.md) for the format.
