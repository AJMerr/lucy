# Memory

Lucy remembers you across chats. When you tell her something worth keeping (your editor, your
dog's name, the project you're on, who your sister is), she saves it and every later chat starts
with it, in the terminal, the window, the sprite and on your other machines.

## How she saves things

She decides by herself, without being asked, and shows it when she does: "Remembered “Edits
code in Neovim”" in the terminal, and "remembered" with **Undo** in the window.

- **Sensitive facts ask first.** Health, money, relationships, your home address and ID numbers
  get the usual Allow / Deny card before they're saved.
- **Secrets are never saved.** Anything that looks like an API key, token, password or card
  number is refused, whatever she tries. Keys belong in Lucy's keyring (Settings → Models & keys,
  or `lucy auth set`).
- **She keeps it current.** When something changes she replaces the old fact, and "forget that"
  in any chat removes one.
- **Notes, not orders.** She treats what she remembers as facts about you, never as
  instructions, so text slipped into a web page or a file can't plant standing orders in her
  memory. A chat from another machine needs your approval here before it changes her memory.

## Seeing and changing it

- **Settings → Memory** lists the facts under their headings, with Edit and Forget for each, and
  "Edit as text" for the whole file.
- **`lucy memory`** prints it; **`lucy memory edit`** opens it in your editor (`$VISUAL` or
  `$EDITOR`).
- It's a plain Markdown file, `~/.local/share/lucy/memory.md` (macOS: `~/Library/Application
  Support/lucy/`, Windows: `%APPDATA%\lucy\`), so any editor works too:

  ```markdown
  # Lucy's memory

  ## About you
  - Name: Sam
  - Has a dog called Biscuit

  ## Preferences
  - Likes short answers
  - Edits code in Neovim

  ## People
  - Sister: Maya, birthday 12 March

  ## Projects
  - Building a cabin in the woods
  ```

  Facts are `- ` lines under `## ` headings; anything else you write is left alone.

## When memory gets big

While memory is small (about 8 KB, a couple of thousand words) all of it goes into every chat.
Past that, she always gets **About you** and **Preferences**, plus the facts that match what
you're talking about, and she can search the rest herself with her `recall` tool.

Matching is by keyword out of the box: no extra model, works offline. To match by meaning
instead ("my pet" finds "Has a dog"), choose an embedding model in Settings → Memory, e.g. in
Ollama:

```sh
ollama pull nomic-embed-text
```

Add it as a profile (Models & keys: Ollama, model `nomic-embed-text`) and pick it under **Find
memories by meaning with**, or in `config.toml`:

```toml
[profiles.embed]
provider = "openai_compat"
base_url = "http://localhost:11434/v1"
model = "nomic-embed-text"

[memory]
embedding_profile = "embed"
```

Each fact is embedded once and kept in `memory.embeddings.json` next to `memory.md`.

## Incognito chats

An incognito chat neither reads nor saves memories, and is deleted (with any pictures sent in
it) when you leave it: when you start another chat, open a different one, or quit.

- Terminal: `lucy --incognito`, or `/new incognito` in a chat.
- Window: **New incognito chat** under New chat. It's marked "incognito" while it's open.

## Turning memory off

Settings → Memory → **Let Lucy remember things about you**, or:

```toml
[memory]
enabled = false
```

She then neither reads nor changes `memory.md`.
