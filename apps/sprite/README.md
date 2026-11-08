# Lucy's desktop sprite

A small transparent window that sits above your other windows in the bottom-right corner,
draws Lucy from a sprite pack, and talks with you in speech bubbles. See
[ADR 0005](../../docs/adr/0005-sprite-tauri-and-packs.md).

> **Status:** milestone 2 is done: she shows up, animates, talks in bubbles, opens her whole chat
> in a window, starts at login if you like, and comes as installers and a Nix package.

## Talking to her

Click her (or press your hotkey) and type in the box above her; Enter sends, Esc closes.
Her reply streams into a speech bubble (bold, italics, code and lists render) and stays up
until you send your next message, or until you hide it with ×. To have replies fade by
themselves, set `[sprite] bubble_timeout_secs` to a number of seconds. Past about 8 lines it
stops with "more…", which opens her chat window. If she needs your go-ahead for a tool (a
command, a file change, going online) the bubble asks, with **Allow**, **Always** (for the rest of the chat) and **Deny**, and
"show changes" for edits.

## Her chat window

"more…" or `lucy sprite chat` opens an ordinary window with her whole chat: what you said, her
replies (with Markdown) and the tools she ran, streaming as she goes. Type at the bottom (Enter
sends, Shift+Enter starts a new line), **Stop** cuts a reply short, and approval requests show
there with the same buttons. Opened partway through a reply, it picks up from what she's said
so far. While it's open her replies go there rather than into bubbles; she still acts them out.

Her mood follows the conversation: `listening` while you type or she waits for an answer,
`thinking` until her reply starts, `talking` while it streams, then `happy`, or `error` if it
failed. She keeps one chat of her own, titled "Sprite" in `/sessions`, which carries on after
restarts; `lucy -r <id>` continues it in a terminal, and those replies show in her bubble too.
She uses the same core as `lucy`: this machine's (started if needed) or, after
`lucy remote connect`, the remote one.

## Hotkey

`lucy sprite ask` opens her input box (starting her first if she isn't running), and
`lucy sprite toggle` hides or shows her. Bind them to keys:

- **Wayland** (apps can't grab global keys there), in your compositor's config:
  - Hyprland: `bind = CTRL SHIFT, SPACE, exec, lucy sprite ask`
  - Sway: `bindsym Ctrl+Shift+space exec lucy sprite ask`
  - KDE / GNOME: a custom shortcut in System Settings running `lucy sprite ask`
- **X11, macOS, Windows:** `[sprite] hotkey` (default `Ctrl+Shift+Space`) works by itself;
  set it to `""` to turn it off.

While the input box is open she takes the keyboard, so typing goes to her until you send or
press Esc.

## Installing

- **A release installer** (`.deb`, `.rpm`, `.AppImage`, `.dmg`, `.msi`) has both her and `lucy`.
- **Nix:** `nix profile install github:AJMerr/lucy`, or `programs.lucy.sprite` in the
  home-manager module (see the [main README](../../README.md)).
- **From source**, below.

To start her at login, set `[sprite] autostart = true` and run `lucy service install` (or, with
home-manager, `programs.lucy.sprite.autostart = true`).

## Running it

It's a [Tauri](https://tauri.app) app: a Rust side (`src-tauri/`, here) and web pages in
[`apps/ui`](../ui), shared with Lucy's coming GUI (`index.html` for her, `chat.html` for her
chat window). It has its own Cargo workspace, because it needs WebKitGTK and friends on Linux,
which the rest of Lucy doesn't.

```sh
nix develop                    # from the repo root; or install Tauri's prerequisites yourself
cd apps/ui
npm ci
npm run build                  # the pages, into apps/ui/dist
cd ../sprite/src-tauri
cargo build --release --features tauri/custom-protocol
cp target/release/lucy-sprite ~/.cargo/bin/   # next to `lucy`, or anywhere on PATH

lucy sprite                    # start her
lucy sprite ask                # open her input box (bind this to a key)
lucy sprite chat               # open her chat window
lucy sprite toggle             # hide or show her
lucy sprite quit               # send her off
```

`cargo tauri dev` (also in the dev shell, from `apps/sprite`) runs her with the pages served
live from Vite.
Starting `lucy-sprite` again while one is running just hands its command to the running one.

To build the installers locally, run `npm ci` here and in `apps/ui`, put a `lucy` build at
`src-tauri/binaries/lucy-<target triple>` (e.g. `lucy-x86_64-unknown-linux-gnu`) and run
`npm run tauri -- build --config src-tauri/tauri.bundle.conf.json --bundles deb`. The release
workflow does this for every platform.

## Where she appears

- **Wayland with layer shell** (Hyprland, Sway, KDE Plasma, niri, river, …): on the overlay
  layer, above everything, without taking focus. No window rules needed.
- **X11, macOS, Windows:** a small always-on-top window without a taskbar entry.
- **GNOME on Wayland** has no layer shell and doesn't let apps place themselves, so she's an
  ordinary floating window there.

On Linux she sets `WEBKIT_DISABLE_DMABUF_RENDERER=1` (unless you've set it), because
WebKitGTK's DMA-BUF renderer crashes or draws nothing on some GPUs under Wayland, NVIDIA's
included. If your compositor has layer shell but she misbehaves on it, set
`LUCY_SPRITE_NO_LAYER_SHELL=1` to get an ordinary window instead.

## Her look

She draws the pack named by `[sprite] pack` in `config.toml`, scaled by `[sprite] scale`.
Packs live in `~/.local/share/lucy/packs/<name>/` (your data folder; `LUCY_DATA_DIR` moves it),
and `placeholder` is built in. To make one, copy [`packs/placeholder`](../../packs/placeholder)
and swap the frames: `pack.toml` lists PNG frames for `idle`, `thinking` and `talking`
(required) and `happy`, `error` and `listening` (optional; they show `idle` if missing). If a
pack has a mistake, she says what's wrong in `sprite.log` (next to `core.log`) and shows the
placeholder.

## Tests

```sh
npm test                       # in apps/ui: animation, what she shows, bubble text, transcripts
cargo test -p lucy-packs       # from the repo root: pack loading and checks
```
