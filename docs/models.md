# Models

Lucy works with any OpenAI-compatible API (Ollama, OpenAI, OpenRouter, Qwen/DashScope,
LM Studio, llama.cpp, vLLM…) and with Anthropic. Each model you set up is a **profile**: a name,
the provider, its address, the model's ID and the name of its key in your keyring.

## Adding a model

You don't need to know the model's ID or the provider's address.

- **In the app:** Settings → Models & keys → **Add a model…**
- **In a terminal:** `lucy config add-model`

Both work the same way:

1. **Pick a provider:** Ollama on this machine, Qwen Cloud, OpenRouter, Anthropic, OpenAI, or
   something else (you type its address).
2. **Paste its API key** if it takes one. The key goes straight into your OS keyring and is never
   shown again. Leave it empty if you saved it before.
3. **Choose a model** from the ones the provider actually offers. Long lists can be searched,
   and models that can see pictures say so. If the provider doesn't list its models, type the ID.
4. **Try it:** Lucy sends it a short test message and shows the reply. If it doesn't answer you
   can still save it, e.g. to fix the key later.
5. **Save** it under a name (one is suggested from the model's ID). Whether it can see pictures,
   and how much it can read at once (its context size), are filled in when the provider says.

The first-run setup (`lucy config init`, or the app's welcome screen) sets up your first model;
add more any time.

## Switching models

- **One chat:** the model picker in the chat's header, or `/model <name>` in the terminal.
  `/model` on its own lists your profiles.
- **The default** (new chats, and the sprite unless you've switched hers): **Make default** next
  to the picker, `/model <name> --default`, or the default profile in Settings → Models & keys.

The sprite shows which model she's on in her input box.

## Long chats

Every model can only read so much at once (its context window). Lucy keeps chats within it:

- **She learns each model's context size** from the provider (Ollama, OpenRouter, Anthropic and
  others that list it), or from `context_tokens` in the profile, which wins.
- **Near the limit** (three quarters full), she replaces older turns with a summary she writes
  herself, and keeps the recent ones word for word. The summary is saved with the chat, so it's
  only written once. The chat itself keeps every message: you can still scroll back and read it
  all, and a line in the window marks where the summary starts (**Show summary** opens it).
- When a model's size isn't known, nothing is summarised and the provider decides what happens
  to a chat that's too long.

**Ollama:** it reports the model's full context size, but runs models with a smaller window
unless you raise it (`OLLAMA_CONTEXT_LENGTH`, or `num_ctx` in a Modelfile). Set
`context_tokens` in the profile to the window Ollama actually uses, so Lucy summarises before
Ollama starts cutting off the start of the chat:

```toml
[profiles.local]
provider = "openai_compat"
base_url = "http://localhost:11434/v1"
model = "qwen3:14b"
context_tokens = 8192
```

### /clear and /compress

These work in the terminal, the window and the sprite:

- **`/clear`:** she starts afresh from here, as if the chat were new. What came before stays saved
  and visible above a "cleared" line, but she no longer reads it.
- **`/compress`:** summarise the chat so far now, rather than waiting until it's nearly full.
  Handy before a long task, or to make a slow local model quicker.

Neither works while she's replying.

## Keys

Keys never go in config.toml. They're kept in your OS keyring (Secret Service, Keychain or
Credential Manager): set them in Settings → Models & keys, or with `lucy auth set <key name>`.
`LUCY_KEY_<NAME>` overrides one, and vendor variables like `ANTHROPIC_API_KEY` are used as a
fallback. `lucy auth status` shows where each comes from, without printing it.

## By hand

Profiles live in config.toml (`lucy config path` prints where);
[config.example.toml](config.example.toml) documents every setting. Settings → Models & keys
edits them too: the address, model, key name, prices for agent cost caps, whether to offer the
model tools, and whether it can see pictures.
