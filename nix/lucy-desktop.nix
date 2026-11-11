# `lucy-desktop`: Lucy's desktop app (sprite, main window, tray), a Tauri app (Rust side in
# apps/desktop, pages in apps/ui). Linux only here; macOS and Windows get Tauri bundles from
# the release workflow.
{
  lib,
  rustPlatform,
  importNpmLock,
  nodejs_24,
  pkg-config,
  wrapGAppsHook3,
  makeDesktopItem,
  copyDesktopItems,
  alsa-lib,
  dbus,
  stdenv,
  glib-networking,
  gtk3,
  gtk-layer-shell,
  libayatana-appindicator,
  librsvg,
  libsoup_3,
  webkitgtk_4_1,
  lucy,
  version,
}:

rustPlatform.buildRustPackage {
  pname = "lucy-desktop";
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
      (lib.fileset.difference ../apps/desktop/src-tauri (
        lib.fileset.unions [
          (lib.fileset.maybeMissing ../apps/desktop/src-tauri/target)
          (lib.fileset.maybeMissing ../apps/desktop/src-tauri/gen)
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

  cargoRoot = "apps/desktop/src-tauri";
  buildAndTestSubdir = "apps/desktop/src-tauri";
  cargoLock.lockFile = ../apps/desktop/src-tauri/Cargo.lock;
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
    alsa-lib
    dbus
    glib-networking
    gtk3
    gtk-layer-shell
    libayatana-appindicator
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

  # She starts the core with `lucy`, found next to her or on PATH. The tray icon loads
  # AppIndicator at runtime, by name, so it goes on the library path; so does the C++ runtime
  # for her voice engine, which she downloads and loads at runtime too.
  preFixup = ''
    gappsWrapperArgs+=(--prefix PATH : ${lib.makeBinPath [ lucy ]})
    gappsWrapperArgs+=(--prefix LD_LIBRARY_PATH : ${lib.makeLibraryPath [ libayatana-appindicator stdenv.cc.cc.lib ]})
  '';

  desktopItems = [
    (makeDesktopItem {
      name = "lucy-desktop";
      desktopName = "Lucy";
      comment = "Lucy, your AI assistant, on the desktop";
      exec = "lucy-desktop";
      icon = "lucy-desktop";
      categories = [ "Utility" ];
    })
  ];
  postInstall = ''
    install -Dm644 apps/desktop/src-tauri/icons/icon.png $out/share/icons/hicolor/128x128/apps/lucy-desktop.png
  '';

  meta = {
    description = "Lucy's desktop app: her sprite, her main window (chats and settings) and a tray icon";
    homepage = "https://github.com/AJMerr/lucy";
    license = with lib.licenses; [ mit asl20 ];
    mainProgram = "lucy-desktop";
    platforms = lib.platforms.linux;
  };
}
