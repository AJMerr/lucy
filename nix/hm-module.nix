# home-manager module: `programs.lucy`. Installs Lucy (and her sprite), optionally writes
# config.toml, and runs the core and the sprite as systemd user services.
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
        `lucy config init` to write. When set, the file is read-only: `lucy config init` and
        `lucy mcp add|remove` say so instead of changing it. API keys never go here; keep them
        in the keyring with `lucy auth set <name>`.
      '';
    };

    service.enable = lib.mkEnableOption ''
      the core as a systemd user service, from login until stopped (what
      `lucy service install` sets up otherwise; don't use both). It sees the user manager's
      environment, not your shell's'';

    sprite = {
      enable = lib.mkEnableOption "Lucy's desktop sprite (`lucy-sprite`)";

      package = lib.mkOption {
        type = lib.types.package;
        default = packages.lucy-sprite;
        defaultText = lib.literalExpression "lucy.packages.\${system}.lucy-sprite";
        description = "The `lucy-sprite` package.";
      };

      autostart = lib.mkEnableOption "starting the sprite with your graphical session";
    };
  };

  config = lib.mkIf cfg.enable (
    lib.mkMerge [
      {
        assertions = [
          {
            assertion = cfg.sprite.autostart -> cfg.sprite.enable;
            message = "programs.lucy.sprite.autostart needs programs.lucy.sprite.enable.";
          }
        ];
        home.packages = [ cfg.package ] ++ lib.optional cfg.sprite.enable cfg.sprite.package;
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

      (lib.mkIf cfg.sprite.autostart {
        systemd.user.services.lucy-sprite = {
          Unit = {
            Description = "Lucy's desktop sprite";
            PartOf = [ "graphical-session.target" ];
            After = [ "graphical-session.target" ] ++ lib.optional cfg.service.enable "lucy-core.service";
          };
          Service = {
            ExecStart = lib.getExe cfg.sprite.package;
            # `lucy sprite quit` leaves her gone; a crash brings her back.
            Restart = "on-failure";
            RestartSec = 5;
          };
          Install.WantedBy = [ "graphical-session.target" ];
        };
      })
    ]
  );
}
