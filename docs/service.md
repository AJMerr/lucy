# Running at login

`lucy` and the app start Lucy's core in the background when it's needed, and the core exits after
30 idle minutes (it stays up while reminders or tasks are scheduled). To keep it running from
login instead, so scheduled tasks run after a reboot and your other machines can always reach
it, install it as a service: Settings → Service & startup in the app, or

```sh
lucy service install      # start now and at every login
lucy service status       # installed? running how, which version, busy with what?
lucy service restart      # after upgrading Lucy, to run the new version
lucy service uninstall    # back to starting on demand
lucy core stop            # stop whichever core is running (asks if Lucy is mid-reply; -f cancels)
```

It's a systemd user unit on Linux (`~/.config/systemd/user/lucy-core.service`), a launchd agent
on macOS (`~/Library/LaunchAgents/io.github.ajmerr.lucy.core.plist`), and a value under
`HKCU\…\CurrentVersion\Run` on Windows. None of them needs admin rights.

The service runs with the `PATH` and `LUCY_*` settings of the shell you installed it from (so
`run_command` and MCP servers find the same programs); run `install` again if those change. API
keys aren't copied into it, so keep them in the keyring. On Windows the service sees your user
environment variables instead, and a console window may flash briefly at login.

On NixOS and other home-manager setups, use `programs.lucy.service.enable` instead; see
[Nix and home-manager](nix.md).

## After an upgrade

A core that was already running keeps the old version until it restarts. When a newer `lucy`
finds an older core, it restarts it by itself as long as it isn't in the middle of a reply, or
says how to. The app shows a banner with a **Restart it** button.

## The app at login

Tick "Start Lucy at login" in Settings → Sprite (or set `[app] autostart = true` in config.toml
and run `lucy service install` again). That adds the app to your login items, starting in the
background (the sprite, if she's on, and the tray): an autostart entry in `~/.config/autostart/`
on Linux, a launch agent on macOS, a second `Run` value on Windows. Untick it (or set it back to
`false` and install again), or run `lucy service uninstall`, to take it off.
