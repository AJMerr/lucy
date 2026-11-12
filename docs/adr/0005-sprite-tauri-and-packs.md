# 5. Desktop sprite: Tauri app with user-supplied sprite packs

Date: 2026-11-05 · Status: Accepted

## Context

Lucy should eventually appear as an animated character on the desktop, replying in chat
bubbles, toggleable. A PWA cannot draw a transparent always-on-top overlay on the desktop;
Electron can but costs ~150 MB+ RAM for an always-on sprite.

## Decision

- The sprite is a **Tauri** app (`apps/sprite`): transparent, frameless, always-on-top,
  click-through except over the character. On Hyprland/Wayland, placement uses a window rule.
- **Interaction:** global hotkey or click opens an input box; replies stream into a bubble that
  fades; long replies collapse to "more…" which opens a full chat window.
- **Art is never committed.** Characters come from **sprite packs** the user configures, found
  in `<data dir>/lucy/packs/`. The repo ships only `packs/placeholder`.
- Pack format: a folder with `pack.toml`. `kind = "frames"` (PNG frames per state) now;
  `kind = "live2d"` later, rendered in the same webview. All renderers share the same state
  names: `idle`, `thinking`, `talking` (required); `happy`, `error`, `listening` (optional,
  fall back to `idle`).

## Consequences

- Anyone can make Lucy look like anything; the code stays character-neutral and safe to make
  public later.
- The renderer is an interface in the webview so Live2D slots in without core changes.

## Implementation notes (v0.4.0, the sprite)

- **Placement is compositor-agnostic.** On Wayland with the layer-shell protocol the sprite
  is an overlay-layer surface anchored bottom-right (any wlroots compositor, KDE, …), so no
  Hyprland window rules are needed. gtk-layer-shell must be set up before GTK realizes a
  window, and Tauri's is realized at creation, so the webview's container is moved into a fresh
  layer-shell window and Tauri's own window stays hidden. Elsewhere it's an always-on-top,
  undecorated window placed in the work area's corner.
- WebKitGTK's DMA-BUF renderer is turned off (`WEBKIT_DISABLE_DMABUF_RENDERER=1`) unless the
  user set it: on NVIDIA under Wayland it crashes (protocol error 71) or draws nothing.
- **Packs** are loaded and checked by `crates/lucy-packs` (outside the sprite, so its tests run
  without Tauri): user packs in `<data dir>/packs/<name>` first, then the built-in placeholder,
  compiled in. Frames must be PNGs inside the pack folder; unknown state names are rejected as
  typos; missing optional states become copies of `idle`. A pack that fails to load is logged
  and replaced by the placeholder.
- The page gets the pack as data URLs and draws it on a canvas; `src/animator.ts` decides the
  frame (fps, loops, one-shot states handing over to `then`).
- `lucy-sprite` is a single instance (tauri-plugin-single-instance): `lucy sprite <command>`
  runs it, and a second launch forwards its command to the running one.
- The app is its own Cargo workspace, so the main workspace builds and tests without
  WebKitGTK.

## Implementation notes (v0.4.0, talking)

- **Her chat:** one ongoing session titled "Sprite", its ID kept in `<data dir>/sprite-session`
  and recreated if it's gone. The sprite connects through `lucy-cli`'s connection logic, so it
  uses the same core `lucy` would (local, started on demand, or remote) and subscribes to all
  events, keeping only her chat's. Messages go with no working folder, so tools use the core's
  default.
- **What she shows** is a pure reducer (`src/view.ts`) from core events and user actions to
  animation, bubble, input box and approval; timers enter as ticks. Bubble text is a light,
  always-escaped Markdown subset capped at ~8 lines (`src/bubble.ts`). Approval summaries are
  built from text nodes, never HTML.
- **Layout:** everything hangs from the bottom-right corner. On a layer surface the window has
  a fixed size with room for the bubble (WebKit doesn't repaint a layer surface that grows),
  and an input region limits clicks to her figure, the bubble and the input box; the keyboard
  is taken (`Exclusive`) only while the input box is open. Elsewhere the window resizes to its
  content. Regions are sent as offsets from the bottom-right, so they hold at either size.
- **Repaint workaround:** without the DMA-BUF renderer, WebKitGTK repaints only damaged areas
  into alternating buffers, so with an animated canvas the rest of the page vanished on every
  other frame. The canvas now draws only when the frame changes, and each draw (and each UI
  update) nudges the root background imperceptibly so the whole page is repainted.
- **Commands and hotkey:** `lucy sprite ask|toggle|quit` reach the running sprite through the
  single-instance plugin (`ask` and `toggle` start her if needed). `[sprite] hotkey` registers
  a global shortcut where apps may (not on Wayland, where the compositor binds `lucy sprite
  ask`). `LUCY_SPRITE_NO_LAYER_SHELL=1` forces an ordinary window.

## Implementation notes (v0.4.0, around her)

- **Chat window:** a second, ordinary Tauri window (`chat.html`), made on demand by "more…" or
  `lucy sprite chat`. Its transcript is a pure reducer (`src/transcript.ts`) from her saved
  messages (`sessions.load`) and live events. The Rust side follows her reply in progress, so a
  window opened mid-reply starts from what she's said. Both pages get the same `core-event`s;
  while the window is open the sprite's reducer keeps replies and approvals out of the bubble.
- **Autostart:** `[sprite] autostart` is applied by `lucy service install`, next to the core's
  service: an XDG autostart `.desktop` entry on Linux (run by GNOME, KDE and systemd-based
  sessions like uwsm), a launchd agent without `KeepAlive` on macOS (quitting her keeps her
  gone), and a second `Run` value on Windows. Not started on install; `lucy sprite` does that.
- **Nix:** `.#lucy` and `.#lucy-sprite` (Linux; wrapped with GTK's environment and `lucy` on
  `PATH`), and a home-manager module whose units are plain systemd user services. The core's
  unit pins `LUCY_SOCKET=%t/lucy/core.sock`; `lucy` resolves `%t` so it recognises the unit as
  its service. A home-manager `config.toml` is a link into the store, which `lucy` refuses to
  rewrite with a message rather than a permission error.
- **Bundles:** `tauri.bundle.conf.json` is merged in only for release builds, because it names
  the `lucy` sidecar (`binaries/lucy-<triple>`), which ordinary builds don't have. Published
  releases trigger `.github/workflows/release.yml`, which builds `lucy`, bundles per platform
  and attaches the results. Nothing is signed yet.

## Implementation notes (v0.5.0, one app)

- The sprite became part of Lucy's desktop app: "Lucy", the `lucy-desktop` program in
  `apps/desktop` (identifier `io.github.ajmerr.lucy`), with the sprite, the main window and a
  tray icon. Its pages live in `apps/ui`. `lucy app` and `lucy sprite …` both start or drive it.
- Started with no command (the app menu) it opens the main window; with `background` (login
  entries) only the sprite, and only if `[sprite] enabled`. Commands about her (`sprite`, `ask`,
  `toggle`) show her even when she's off at start.
- `[app] autostart` replaces `[sprite] autostart` (still read). Its login entries are
  `lucy-desktop.desktop`, `io.github.ajmerr.lucy.desktop` and the `LucyDesktop` Run value;
  installing or uninstalling also removes v0.4.0's `lucy-sprite` ones. Nix: `.#lucy-desktop`
  and `programs.lucy.desktop.*` (the old `sprite.*` options are renames).
- The layer-shell namespace stays `lucy-sprite`, since it names her layer and compositor rules
  may match it.

## Implementation notes (v1.0.0, her own look)

- The repo now ships one character pack, `packs/default`: an original look made for Lucy with
  AI image models (credited in its README), compiled in like the placeholder and the default
  for `[sprite] pack`. Other art still isn't committed; personal packs stay in the data folder.
- A pack that fails to load is replaced by `default` rather than the placeholder.
- `placeholder` stays built in as the plain pack to copy when making your own.
