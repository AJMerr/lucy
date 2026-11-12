# MCP servers

MCP servers give Lucy more tools: your email and calendar, GitHub, a web browser, databases,
and hundreds more. Some are programs she starts on your computer; others are services she
reaches by URL. Their tools follow the same approval rules as hers: ones the server marks
read-only just run, and everything else asks you first.

## Adding one

### Ready-made

Settings → MCP servers → **Add a ready-made one**:

| | What it gives her | What it needs |
| --- | --- | --- |
| **Google Workspace** | Gmail, Calendar, Drive, Docs, Sheets and more ([workspace-mcp](https://github.com/taylorwilsdon/google_workspace_mcp)) | [uv](https://docs.astral.sh/uv/), and a Google OAuth client of your own (below) |
| **GitHub** | issues, pull requests, code and Actions (GitHub's own server, by URL) | a GitHub personal access token, saved as the key `github` |
| **Web browser** | a real browser she can open pages in, click and type ([Playwright MCP](https://github.com/microsoft/playwright-mcp)) | [Node.js](https://nodejs.org); the first run downloads a browser |

Save, then set any keys it needs under Models & keys. New chats can use its tools.

### Pasting a command

Most servers' instructions give a command to run, like
`npx -y @modelcontextprotocol/server-filesystem ~/Notes`. Type a name in Settings → MCP servers,
paste the command under it and press Add: it's split into the program, its arguments and any
`NAME=value` variables in front, quotes and all. Check what it filled in, then Save.

If a variable is a secret (a token or API key), don't leave it in the variables: put it under
**Variables from the keyring** as `NAME=key name` instead, and set that key under Models & keys.
config.toml then only holds the key's name.

### In a terminal

```sh
lucy auth set github                     # store the token in the keyring
lucy mcp add github --key GITHUB_PERSONAL_ACCESS_TOKEN=github \
  -- npx -y @modelcontextprotocol/server-github
lucy mcp add linear --url https://mcp.linear.app/mcp     # a remote one; offers to sign in
lucy mcp add                             # or answer a few questions instead
lucy mcp list                            # which servers start, and how many tools each has
lucy mcp remove github
lucy mcp login linear                    # sign in again; `lucy mcp logout linear` forgets it
```

`lucy mcp add` keeps your config's comments and layout.

## How they work

- A server's tools show up as `<server>__<tool>` (e.g. `github__create_issue`).
- Servers start the first time a chat needs tools and keep running until the core stops; one that
  crashes is restarted on next use.
- Tools the server labels read-only run by themselves (with the same path checks as Lucy's own);
  everything else asks. Set `github = "ask"` (or `"deny"`) under `[tools.rules]` to override a
  whole server; see [Tools](tools.md#rules).
- **Remote servers** are reached over streamable HTTP. Sign-in uses OAuth: Lucy registers herself
  with the server, opens your browser, catches the redirect on localhost, and keeps the tokens in
  your keyring, refreshing them when they expire. If a refresh is refused, `/tools` and
  `lucy mcp list` say to sign in again (Sign in in Settings, or `lucy mcp login`). Servers that
  use API keys take a bearer key (`--bearer-key`) or header keys (`--header-key HEADER=keyname`)
  instead. A remote server's file paths aren't on your machine, so the home-folder checks don't
  apply to its read-only tools.
- In config.toml they're under `[mcp.servers.<name>]`; see
  [config.example.toml](config.example.toml).

## Google Workspace: making an OAuth client

Google doesn't let apps reach your Gmail or Calendar without an OAuth client registered to
someone. That's you: it takes about ten minutes, once, and nothing is shared with anyone else.

1. Go to the [Google Cloud console](https://console.cloud.google.com/) and create a project (any
   name, e.g. "Lucy").
2. **Enable the APIs** you want her to use: under APIs & Services → Library, enable the Gmail,
   Google Calendar, Google Drive, Google Docs and Google Sheets APIs (any you leave out just won't
   work).
3. **Set up the consent screen** (APIs & Services → OAuth consent screen, or Google Auth
   Platform): user type **External**, your own email as the support and developer contact, and
   add yourself under **Test users**. It can stay in testing; only test users can sign in.
4. **Create the client:** Credentials → Create credentials → OAuth client ID → application type
   **Desktop app**. Copy the client ID and client secret.
5. In Lucy, Settings → Models & keys: set the key **google-client-id** to the client ID and
   **google-client-secret** to the secret. (They appear there once the Google Workspace server
   is saved; or run `lucy auth set google-client-id` and `lucy auth set google-client-secret`.)

The first time she uses a Google tool, the server asks you to sign in with Google: open the link
she passes on (or the browser window that opens), pick your account and allow access. The server
listens for Google's answer on `http://localhost:8000`, so that port needs to be free. It keeps
its own sign-in from then on.

The preset starts `uvx workspace-mcp --tool-tier core`, the most used tools. For more, change the
arguments to `--tool-tier extended` or `complete`, or pick services with
`--tools gmail calendar drive`; fewer tools suit smaller local models better.

## GitHub: making a token

On GitHub, Settings → Developer settings → Personal access tokens → **Fine-grained tokens** →
Generate new token. Pick the repositories she may use and the permissions she needs (read-only
contents and issues is a safe start). Then save it in Lucy as the key **github** (Settings →
Models & keys, or `lucy auth set github`).
