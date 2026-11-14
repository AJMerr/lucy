# The first five minutes

## 1. Set her up

**In the app:** open Lucy. With nothing set up yet, her window starts with setup: pick a
provider (Ollama on this computer, OpenRouter, Anthropic, OpenAI, Qwen Cloud, or any other
OpenAI-compatible address), check the model, paste its API key if it takes one, and say what
she should call you.

**In a terminal:**

```sh
lucy config init     # the same questions
lucy                 # start chatting
```

Keys go straight into your OS keyring, never into a file. You can add more models later
(Settings → Models & keys → Add a model…, or `lucy config add-model`); see [Models](models.md).

## 2. Say hello

Type a message and she answers as it streams in. A few things to try:

- **"What's in my Downloads folder?"** She asks first, then lists it: she reads freely only in
  the folder you're working in and folders you let her (`[tools] readable`, or Settings → Chat &
  agent), so a file you never mentioned stays private. **Always** covers the rest of the chat.
- **"Make a file called notes.md with a shopping list."** She asks first: anything that changes
  files, runs a command or goes online shows you what she wants to do, with **Allow**,
  **Always** (for this chat) and **Deny**. In the terminal that's `y`, `a` or `n`.
- **"I use Neovim, and my dog is called Biscuit."** She remembers that for every later chat
  (and says so). See [Memory](memory.md).
- **"Remind me in ten minutes to stretch."** She shows you the reminder before saving it, then
  reminds you on time. See [Reminders and tasks](tasks.md).
- **`/plain`** drops her character for plain, factual answers. Run it again to bring her back.

## 3. Meet the sprite

The app puts Lucy on your desktop, in the bottom-right corner. Click her and type; she answers
in a speech bubble, and asks there before using tools. "more…" on a long reply, the tray icon,
or `lucy app` opens her main window with all your chats. Don't want her on screen? Settings →
Sprite. See [The app and the sprite](app.md).

## 4. Pick up where you left off

Every chat is saved. In the window, they're on the left. In a terminal, `lucy -c` continues
your last chat and `lucy -r <id>` resumes any. Chats started in one place show up in the others.

## Where next

- **Let her work on her own:** [Tools and agent mode](tools.md).
- **More tools:** Gmail, Calendar, GitHub, a browser and more through [MCP servers](mcp.md).
- **Talk to her:** [Voice](voice.md).
- **Pictures:** show her one, or ask her to make one. [Pictures](images.md).
- **Keep her running:** [at login](service.md), and [from your other machines](remote.md).
