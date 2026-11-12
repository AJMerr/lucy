# From another machine

Your laptop's `lucy` can use your desktop's core: the same chats, models, keys, memory and MCP
servers, with tools running on the desktop.

## On the machine that runs Lucy

Turn remote access on in Settings → Remote access, or in config.toml, and restart the core:

```toml
[remote]
enabled = true
bind = "100.64.0.7:7077"    # your Tailscale address (or 127.0.0.1 behind an SSH tunnel)
allow_network = true        # needed for anything beyond 127.0.0.1
```

```sh
lucy service restart        # or `lucy core stop`
lucy remote token           # prints the token the other machine needs
```

The core only listens while it runs, so keep it running with [the login service](service.md).

## On the other machine

It needs no config or keys of its own:

```sh
lucy remote connect desktop:7077   # asks for the token; checks it works
lucy                               # now chats with the desktop's core
lucy remote disconnect             # back to this machine's own core
lucy --remote desktop:7077 ask "…" # or just once (LUCY_REMOTE works too)
```

Lucy's app on that machine follows `lucy remote connect` too.

## Things to know

- **No TLS.** Traffic is plain WebSocket, so use a network you trust: Tailscale encrypts it for
  you, or use an SSH tunnel, or put a TLS proxy in front and connect with `wss://`. Binding beyond
  127.0.0.1 needs `allow_network = true`.
- **Tools run on the core's machine**, in `[agent] workspace` if you set one, else its home
  folder; the chat banner says which. The other machine's current folder isn't used.
- **Remote chats are capped at agent level 1** (`[remote] max_level`), so a leaked token can't run
  commands without someone approving them.
- **Remote clients can't change settings, keys or the persona** unless you set
  `[remote] allow_config = true`.
- `lucy remote token --rotate` replaces the token. A remote client can't stop the core.
