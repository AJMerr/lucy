# home-manager module: `programs.lucy`. Installs Lucy (and her desktop app), optionally writes
# config.toml, and runs the core and the desktop app as systemd user services.
self:
{
  config,
  lib,
  pkgs,
  ...
}:

let
  cfg = config.programs.lucy;
  toml = pkgs.formats.toml { };
  packages = self.packages.${pkgs.stdenv.hostPlatform.system};
  # Where `lucy` looks by default; pinned in the core's unit so `lucy service status|restart`
  # and `lucy core stop` know the unit is theirs.
  socket = ''"LUCY_SOCKET=%t/lucy/core.sock"'';
in
{
  # v0.4.0 called the desktop app "the sprite".
  imports = [
    (lib.mkRenamedOptionModule
      [ "programs" "lucy" "sprite" "enable" ]
      [ "programs" "lucy" "desktop" "enable" ]
    )
    (lib.mkRenamedOptionModule
      [ "programs" "lucy" "sprite" "package" ]
      [ "programs" "lucy" "desktop" "package" ]
    )
    (lib.mkRenamedOptionModule
      [ "programs" "lucy" "sprite" "autostart" ]
      [ "programs" "lucy" "desktop" "autostart" ]
    )
  ];

  options.programs.lucy = {
    enable = lib.mkEnableOption "Lucy, a model-agnostic personal AI assistant";

    package = lib.mkOption {
      type = lib.types.package;
      default = packages.lucy;
      defaultText = lib.literalExpression "lucy.packages.\${system}.lucy";
      description = "The `lucy` package (CLI and core).";
    };

    settings = lib.mkOption {
      type = lib.types.nullOr toml.type;
      default = null;
      example = lib.literalExpression ''
        {
          defaults.profile = "local-qwen";
          profiles.local-qwen = {
            provider = "openai_compat";
            base_url = "http://localhost:11434/v1";
            model = "qwen3:14b";
          };
          sprite.pack = "placeholder";
        }
      '';
      description = ''
        Lucy's `config.toml` (see docs/config.example.toml). `null` leaves the file alone, for
        `lucy config init` or the app's first-run setup to write. When set, the file is
        read-only: `lucy config init`, `lucy mcp add|remove` and the app's settings say so
        instead of changing it. API keys never go here; keep them in the keyring with
        `lucy auth set <name>` or the app's settings.
      '';
    };

    service.enable = lib.mkEnableOption ''
      the core as a systemd user service, from login until stopped (what
      `lucy service install` sets up otherwise; don't use both). It sees the user manager's
      environment, not your shell's'';

    desktop = {
      enable = lib.mkEnableOption "Lucy's desktop app (`lucy-desktop`: sprite, window and tray)";

      package = lib.mkOption {
        type = lib.types.package;
        default = packages.lucy-desktop;
        defaultText = lib.literalExpression "lucy.packages.\${system}.lucy-desktop";
        description = "The `lucy-desktop` package.";
      };

      autostart = lib.mkEnableOption ''
        starting the desktop app with your graphical session (in the background: the sprite,
        if `[sprite] enabled`, and the tray)'';
    };
  };

  config = lib.mkIf cfg.enable (
    lib.mkMerge [
      {
        assertions = [
          {
            assertion = cfg.desktop.autostart -> cfg.desktop.enable;
            message = "programs.lucy.desktop.autostart needs programs.lucy.desktop.enable.";
          }
        ];
        home.packages = [ cfg.package ] ++ lib.optional cfg.desktop.enable cfg.desktop.package;
      }

      (lib.mkIf (cfg.settings != null) {
        xdg.configFile."lucy/config.toml".source = toml.generate "lucy-config.toml" cfg.settings;
      })

      (lib.mkIf cfg.service.enable {
        systemd.user.services.lucy-core = {
          Unit.Description = "Lucy core (personal AI assistant)";
          Service = {
            ExecStart = "${lib.getExe cfg.package} core run --service";
            Environment = [ socket ];
            Restart = "on-failure";
            RestartSec = 5;
          };
          Install.WantedBy = [ "default.target" ];
        };
      })

      (lib.mkIf cfg.desktop.autostart {
        systemd.user.services.lucy-desktop = {
          Unit = {
            Description = "Lucy's desktop app";
            PartOf = [ "graphical-session.target" ];
            After = [ "graphical-session.target" ] ++ lib.optional cfg.service.enable "lucy-core.service";
          };
          Service = {
            ExecStart = "${lib.getExe cfg.desktop.package} background";
            # `lucy sprite quit` leaves it gone; a crash brings it back.
            Restart = "on-failure";
            RestartSec = 5;
          };
          Install.WantedBy = [ "graphical-session.target" ];
        };
      })
    ]
  );
}
