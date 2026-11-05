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
