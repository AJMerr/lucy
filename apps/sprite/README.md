# Lucy's desktop sprite

A small transparent window that sits above your other windows in the bottom-right corner
and draws Lucy from a sprite pack. See [ADR 0005](../../docs/adr/0005-sprite-tauri-and-packs.md).

> **Status:** milestone 2, slice 2a. She shows up and animates. Click her and she's happy.
> Talking to her (input box, reply bubbles, approvals), hotkeys, the chat window, autostart
> and packages come in the next slices.

## Running it

It's a [Tauri](https://tauri.app) app: a Rust side (`src-tauri/`) and a TypeScript page
(`src/`). It has its own Cargo workspace, because it needs WebKitGTK and friends on Linux,
which the rest of Lucy doesn't.

```sh
nix develop                    # from the repo root; or install Tauri's prerequisites yourself
cd apps/sprite
npm ci
npm run build                  # the page, into dist/
cd src-tauri
cargo build --release --features tauri/custom-protocol
cp target/release/lucy-sprite ~/.cargo/bin/   # next to `lucy`, or anywhere on PATH

lucy sprite                    # start her
lucy sprite quit               # send her off
```

`cargo tauri dev` (also in the dev shell) runs her with the page served live from Vite.
Starting `lucy-sprite` again while one is running just hands its command to the running one.

## Where she appears

- **Wayland with layer shell** (Hyprland, Sway, KDE Plasma, niri, river, …): on the overlay
  layer, above everything, without taking focus. No window rules needed.
- **X11, macOS, Windows:** a small always-on-top window without a taskbar entry.
- **GNOME on Wayland** has no layer shell and doesn't let apps place themselves, so she's an
  ordinary floating window there.

On Linux she sets `WEBKIT_DISABLE_DMABUF_RENDERER=1` (unless you've set it), because
WebKitGTK's DMA-BUF renderer draws nothing on some GPUs, NVIDIA's included.

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
npm test                       # the animation timing (Node's built-in test runner)
cargo test -p lucy-packs       # from the repo root: pack loading and checks
```
