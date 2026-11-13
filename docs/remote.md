# From another machine

Your laptop's `lucy` can use your desktop's core: the same chats, models, keys, memory and MCP
servers, with tools running on the desktop.

## On the machine that runs Lucy

Turn remote access on in Settings → Remote access, or in config.toml. It takes effect straight
away:

```toml
[remote]
enabled = true
bind = "100.64.0.7:7077"    # your Tailscale address (or 127.0.0.1 behind an SSH tunnel)
allow_network = true        # needed for anything beyond 127.0.0.1
```

```sh
lucy remote token           # prints the token, and the certificate fingerprint
```

The core only listens while it runs, so keep it running with [the login service](service.md).

## On the other machine

It needs no config or keys of its own:

```sh
lucy remote connect desktop:7077   # asks for the token and the fingerprint; checks they work
lucy                               # now chats with the desktop's core
lucy remote disconnect             # back to this machine's own core
lucy --remote desktop:7077 ask "…" # or just once (LUCY_REMOTE works too)
```

Lucy's app on that machine follows `lucy remote connect` too.

## Things to know

- **Encrypted.** The core makes its own certificate the first time remote access starts and
  keeps it in the keyring. `lucy remote token` shows its fingerprint and `lucy remote connect`
  pins it, so the laptop only ever talks to that core, with no certificate authority involved. A
  core reinstalled from scratch has a new certificate: connect again with its new fingerprint.
- **Behind a proxy** that adds TLS itself, set `tls = false` and connect with
  `wss://your.host/rpc` (the proxy's certificate is checked the usual way). Plain `ws://` is only
  sent to this machine or a Tailscale address; anywhere else needs `--insecure`.
- **Approvals happen at the desktop.** A remote chat that wants to run a command, change a file
  or go online waits for someone to approve it on the machine running Lucy (the app there shows
  it); from the laptop you can only decline. `[remote] approvals = "remote"` lets the laptop
  approve its own chats too: then whoever holds the token can do anything you can at that
  desk, up to `max_level`. Approving "Always" never carries over to or from a remote chat.
- **Remote chats are capped at agent level 1** (`[remote] max_level`), and so are tasks they
  schedule. Tools run on the core's machine, in `[agent] workspace` if you set one, else its home
  folder; the chat banner says which. Lucy never changes files in home without asking, and never
  touches her own settings or files that run at startup from another machine.
- **Chats are shared, incognito ones aren't.** A remote client sees and continues your chats, but
  never an incognito chat it didn't start. It can only clear, rename, delete or change the
  settings of chats started from another machine.
- **Settings stay put.** Without `[remote] allow_config = true`, a remote client can't change
  settings, keys, the persona, memory or scheduled tasks, or use stored keys to list a
  provider's models; it reads settings with MCP servers' variables and headers hidden. Stored
  keys are only ever sent to the addresses that already use them.
- `lucy remote token --rotate` (on the desktop) replaces the token and disconnects every remote
  client at once. A remote client can't stop the core or sign in to MCP servers.
- **From a browser,** put the token after `#`
  (`https://…/app.html#token=…&url=wss://desktop:7077/rpc`) and add the page's origin to
  `[remote] allowed_origins`. Lucy's own clients need neither.
