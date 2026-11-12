# Nix and home-manager

Lucy's flake has both programs, an overlay, and a home-manager module.

```sh
nix profile install github:AJMerr/lucy      # `lucy` and the app
nix run github:AJMerr/lucy                  # or try it without installing
```

`.#lucy` is just the command; `.#lucy-desktop` is the app (Linux only). `overlays.default` adds
both to your `pkgs`.

## home-manager

```nix
# flake inputs: lucy.url = "github:AJMerr/lucy";
imports = [ lucy.homeModules.default ];
programs.lucy = {
  enable = true;
  service.enable = true;                            # the core from login
  desktop = { enable = true; autostart = true; };  # the app, with your graphical session
  settings = {                                      # optional: config.toml, read-only once set
    defaults.profile = "local-qwen";
    profiles.local-qwen = {
      provider = "openai_compat";
      base_url = "http://localhost:11434/v1";
      model = "qwen3:14b";
    };
  };
};
```

- **`settings`** writes config.toml from Nix ([every setting](config.example.toml)). Once it's
  set the file is read-only: Lucy's Settings, `lucy config init` and `lucy mcp add` say so
  instead of changing it. Leave it out to keep editing config.toml yourself.
- **`service.enable`** runs the core as a systemd user service, instead of
  `lucy service install`.
- **`desktop.autostart`** starts the app with `graphical-session.target`, which uwsm, GNOME and
  KDE sessions reach.

Keys still go in the keyring: run `lucy auth set <name>` (or use Settings → Models & keys) once.

## Local models on NixOS

```nix
services.ollama = {
  enable = true;
  package = pkgs.ollama-cuda;     # NVIDIA GPUs; pkgs.ollama-rocm for AMD
};
```

Then `ollama pull qwen3:14b` and pick "Local Qwen via Ollama" in setup.

## nixpkgs

Lucy isn't in nixpkgs yet; until it is, the flake is the way to get it.
