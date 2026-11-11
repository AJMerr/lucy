# For nixpkgs, as pkgs/by-name/lu/lucy/package.nix. Bump `version`, then set `hash` and
# `cargoHash` from the errors `nix-build -A lucy` gives with them emptied. The flake's own
# package is nix/lucy.nix.
{
  lib,
  rustPlatform,
  fetchFromGitHub,
  versionCheckHook,
}:

rustPlatform.buildRustPackage (finalAttrs: {
  pname = "lucy";
  version = "0.9.0";

  src = fetchFromGitHub {
    owner = "AJMerr";
    repo = "lucy";
    tag = "v${finalAttrs.version}";
    hash = "sha256-j7Kr/sgwFqPwtHpE8EuLr8zWFBYTBRI00XoZ2sGZkdo=";
  };

  cargoHash = "sha256-dnvVy5g5ycFkgBV/peJ+XZCAfCZHDSe3nKPPWpEUi38=";

  cargoBuildFlags = [
    "-p"
    "lucy-cli"
  ];

  # Many tests need a writable home, a keyring or local sockets; upstream CI runs them.
  doCheck = false;

  nativeInstallCheckInputs = [ versionCheckHook ];
  doInstallCheck = true;

  meta = {
    description = "Model-agnostic personal AI assistant: terminal chat, tools, memory and scheduled tasks";
    homepage = "https://ajmerr.github.io/lucy/";
    changelog = "https://github.com/AJMerr/lucy/blob/v${finalAttrs.version}/CHANGELOG.md";
    license = with lib.licenses; [
      mit
      asl20
    ];
    maintainers = [ ];
    mainProgram = "lucy";
    platforms = lib.platforms.unix;
  };
})
