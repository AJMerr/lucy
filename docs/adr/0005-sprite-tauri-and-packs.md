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

## Implementation notes (milestone 2, slice 2a)

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
