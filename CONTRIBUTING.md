# Contributing to Lucy

Thanks for helping. Bug reports, fixes, docs, sprite packs and new ideas are all welcome.

## Before you start

- **Small fixes** (typos, clear bugs): send a pull request straight away.
- **Anything bigger** (a feature, a new setting, a change to how she behaves): open an issue first
  so we can agree on the shape before you spend time on it. Design decisions are recorded in
  [`docs/adr/`](docs/adr/); read the ones near your change.
- **Security problems:** don't open an issue. See [SECURITY.md](SECURITY.md).

## Setting up

The easy way is [Nix](https://nixos.org/download) with flakes on:

```sh
git clone https://github.com/AJMerr/lucy && cd lucy
nix develop          # Rust, Node, and the libraries the desktop app needs
```

Without Nix you need a stable Rust toolchain, Node.js 24
for `apps/ui`, and for the desktop app the
[Tauri prerequisites](https://v2.tauri.app/start/prerequisites/) plus ALSA headers on Linux
(`libasound2-dev`).

## Checks

Everything runs offline, against fake model servers and recorded streams:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace

cd apps/ui && npm ci && npm run typecheck && npm test

cd apps/desktop/src-tauri && cargo clippy --all-targets -- -D warnings   # the desktop app
nix flake check                                                          # packages and the home-manager module
```

CI runs all of these on Linux, macOS and Windows; a pull request needs them green.

## How we work

- **Tests first** at the seams: the core over its socket (with a fake model server), providers
  against recorded streams, the UI with Vitest and a fake core, the CLI through its commands. A bug
  fix comes with the test that would have caught it.
- **Keep the style of the code around you:** naming, comment density, and plain English in
  anything a user reads (errors, settings, docs).
- **Keys never go in files.** API keys live in the OS keyring; config.toml only holds key names.
- **The protocol only grows.** Add fields and methods; don't change or remove existing ones
  without a version bump (`PROTOCOL_VERSION`).
- **Docs move with the code.** A change people will notice updates README.md, the guide in
  `docs/` and CHANGELOG.md.

## Commits and pull requests

Commit messages start with a type: `feat:`, `fix:`, `doc:`, `chore:`, `test:` or `refactor:`,
then say what changed for someone using Lucy, e.g. `feat: /compress in the sprite`. Keep
unrelated changes in separate pull requests.

## Sprite packs

A pack is a folder with a `pack.toml` and its frames; copy `packs/placeholder` to start. Only
share packs whose art you made or may publish: no characters from games, films or shows. See
[ADR 0005](docs/adr/0005-sprite-tauri-and-packs.md).

## Licence

Unless you say otherwise, any contribution you submit for inclusion in Lucy, as defined in the
Apache-2.0 licence, is dual licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE),
without any additional terms or conditions.
