# `lucy-sprite`: Lucy's desktop sprite, a Tauri app (Rust side in apps/sprite, pages in
# apps/ui). Linux only here; macOS and Windows get Tauri bundles from the release workflow.
{
  lib,
  rustPlatform,
  importNpmLock,
  nodejs_24,
  pkg-config,
  wrapGAppsHook3,
  makeDesktopItem,
  copyDesktopItems,
  dbus,
  glib-networking,
  gtk3,
  gtk-layer-shell,
  librsvg,
  libsoup_3,
  webkitgtk_4_1,
  lucy,
  version,
}:

rustPlatform.buildRustPackage {
  pname = "lucy-sprite";
  inherit version;

  src = lib.fileset.toSource {
    root = ../.;
    fileset = lib.fileset.unions [
      # The crates it uses, and the workspace they inherit settings from.
      ../Cargo.toml
      ../Cargo.lock
      ../crates
      ../persona
      ../packs/placeholder
      # Its Rust side, and its pages (apps/ui).
      (lib.fileset.difference ../apps/sprite/src-tauri (
        lib.fileset.unions [
          (lib.fileset.maybeMissing ../apps/sprite/src-tauri/target)
          (lib.fileset.maybeMissing ../apps/sprite/src-tauri/gen)
        ]
      ))
      (lib.fileset.difference ../apps/ui (
        lib.fileset.unions [
          (lib.fileset.maybeMissing ../apps/ui/node_modules)
          (lib.fileset.maybeMissing ../apps/ui/dist)
        ]
      ))
    ];
  };

  cargoRoot = "apps/sprite/src-tauri";
  buildAndTestSubdir = "apps/sprite/src-tauri";
  cargoLock.lockFile = ../apps/sprite/src-tauri/Cargo.lock;
  # Serve the built page from inside the binary.
  buildFeatures = [ "tauri/custom-protocol" ];

  npmRoot = "apps/ui";
  npmDeps = importNpmLock { npmRoot = ../apps/ui; };

  nativeBuildInputs = [
    nodejs_24
    importNpmLock.npmConfigHook
    pkg-config
    wrapGAppsHook3
    copyDesktopItems
  ];
  buildInputs = [
    dbus
    glib-networking
    gtk3
    gtk-layer-shell
    librsvg
    libsoup_3
    webkitgtk_4_1
  ];

  # The pages into apps/ui/dist, which the binary embeds.
  preBuild = ''
    (cd apps/ui && npm run build)
  '';

  # The page tests run in CI (`npm test`); the Rust side has none of its own.
  doCheck = false;

  # She starts the core with `lucy`, found next to her or on PATH.
  preFixup = ''
    gappsWrapperArgs+=(--prefix PATH : ${lib.makeBinPath [ lucy ]})
  '';

  desktopItems = [
    (makeDesktopItem {
      name = "lucy-sprite";
      desktopName = "Lucy Sprite";
      comment = "Lucy, your AI assistant, on the desktop";
      exec = "lucy-sprite";
      icon = "lucy-sprite";
      categories = [ "Utility" ];
    })
  ];
  postInstall = ''
    install -Dm644 apps/sprite/src-tauri/icons/icon.png $out/share/icons/hicolor/128x128/apps/lucy-sprite.png
  '';

  meta = {
    description = "Lucy's desktop sprite: she sits on your desktop and talks in speech bubbles";
    homepage = "https://github.com/AJMerr/lucy";
    # No licence yet (see docs/ROADMAP.md); set it here when there is one.
    mainProgram = "lucy-sprite";
    platforms = lib.platforms.linux;
  };
}
