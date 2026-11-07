# `lucy`: the CLI and the core it runs.
{
  lib,
  rustPlatform,
  version,
}:

rustPlatform.buildRustPackage {
  pname = "lucy";
  inherit version;

  src = lib.fileset.toSource {
    root = ../.;
    fileset = lib.fileset.unions [
      ../Cargo.toml
      ../Cargo.lock
      ../crates
      # Built in: her persona and the placeholder sprite pack.
      ../persona
      ../packs/placeholder
    ];
  };
  cargoLock.lockFile = ../Cargo.lock;
  cargoBuildFlags = [
    "-p"
    "lucy-cli"
  ];

  # CI runs the tests; many need a writable home, a keyring or local sockets.
  doCheck = false;

  meta = {
    description = "Lucy, a model-agnostic personal AI assistant: CLI and core";
    homepage = "https://github.com/AJMerr/lucy";
    # No licence yet (see docs/ROADMAP.md); set it here when there is one.
    mainProgram = "lucy";
    platforms = lib.platforms.unix;
  };
}
