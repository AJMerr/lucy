# `nix flake check`: build a home-manager configuration that uses every part of
# `programs.lucy`, then check what it generates with Lucy herself.
{
  pkgs,
  self,
  home-manager,
}:

let
  home = home-manager.lib.homeManagerConfiguration {
    inherit pkgs;
    modules = [
      self.homeModules.default
      {
        home = {
          username = "lucy-test";
          homeDirectory = "/home/lucy-test";
          stateVersion = "25.05";
        };
        programs.lucy = {
          enable = true;
          settings = {
            defaults.profile = "local-qwen";
            profiles.local-qwen = {
              provider = "openai_compat";
              base_url = "http://localhost:11434/v1";
              model = "qwen3:14b";
            };
            sprite.pack = "placeholder";
          };
          service.enable = true;
          sprite = {
            enable = true;
            autostart = true;
          };
        };
      }
    ];
  };
  files = home.config.xdg.configFile;
in
pkgs.runCommand "lucy-home-manager-module"
  {
    nativeBuildInputs = [ pkgs.lucy ];
    config = files."lucy/config.toml".source;
    core = files."systemd/user/lucy-core.service".source;
    sprite = files."systemd/user/lucy-sprite.service".source;
    packages = map (package: package.name) home.config.home.packages;
  }
  ''
    fail() { echo "FAIL: $*" >&2; exit 1; }
    export HOME=$PWD/home XDG_CONFIG_HOME=$PWD/home/.config XDG_DATA_HOME=$PWD/home/.local/share
    export XDG_RUNTIME_DIR=$PWD/run
    mkdir -p $XDG_CONFIG_HOME/lucy $XDG_CONFIG_HOME/systemd/user $XDG_RUNTIME_DIR
    # Linked from the store, as home-manager does.
    ln -s $config $XDG_CONFIG_HOME/lucy/config.toml
    ln -s $core $XDG_CONFIG_HOME/systemd/user/lucy-core.service

    echo "Both programs are installed: $packages"
    [[ " $packages " == *" lucy-${pkgs.lucy.version} "* ]] || fail "lucy isn't in home.packages"
    [[ " $packages " == *" lucy-sprite-${pkgs.lucy-sprite.version} "* ]] || fail "lucy-sprite isn't"

    echo "Lucy accepts the config it writes"
    lucy mcp list || fail "lucy couldn't load the generated config"

    echo "and won't change it"
    if lucy config init </dev/null >out 2>&1; then fail "config init wrote a managed config"; fi
    grep -q "managed by home-manager" out || { cat out; fail "no home-manager message"; }

    echo "The core's unit runs the core, and lucy knows it as its service"
    grep -qx "ExecStart=${pkgs.lucy}/bin/lucy core run --service" $core || fail "ExecStart: $(cat $core)"
    lucy service status >out 2>&1 || true
    cat out
    grep -q "Service:  installed, starts at login" out || fail "lucy doesn't see the unit as its own"

    echo "The sprite starts with the graphical session"
    grep -qx "ExecStart=${pkgs.lucy-sprite}/bin/lucy-sprite" $sprite || fail "ExecStart: $(cat $sprite)"
    grep -qx "WantedBy=graphical-session.target" $sprite || fail "not wanted by the session"
    grep -qx "After=graphical-session.target" $sprite && grep -qx "After=lucy-core.service" $sprite \
      || fail "ordering: $(cat $sprite)"

    touch $out
  ''
