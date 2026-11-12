# Troubleshooting

- **Something's wrong with the core:** its log is `core.log` in the data folder
  (`~/.local/share/lucy/core.log` on Linux; see [where things live](configuration.md#where-things-live)).
- **You upgraded Lucy but the old behaviour sticks:** the previous core is still running. `lucy`
  and the app say so; run `lucy service restart` (or `lucy core stop` if you don't use the
  service), or press **Restart it** in the app's banner.
- **"Couldn't list its models" when adding one:** check the address (Ollama is
  `http://localhost:11434/v1`) and the key. You can still type the model's ID by hand.
- **She forgets the start of a long chat on Ollama:** set `context_tokens` in the profile to the
  window Ollama runs with. See [Models](models.md#long-chats).
- **A model ignores tools or writes tool calls as text:** some models can't call tools. Add
  `tools = false` to its profile, or pick a model that can (Qwen3, Llama 3.1+, GPT, Claude…).
- **The service won't start:** `lucy service status`, then the log. On Linux,
  `systemctl --user status lucy-core` and `journalctl --user -u lucy-core` show what systemd saw.
- **Keyring errors on Linux:** you need a Secret Service provider (e.g. GNOME Keyring or
  KWallet), or set `LUCY_KEY_<NAME>` instead.
- **"config.toml is managed by home-manager":** `programs.lucy.settings` owns it. Change Lucy's
  settings in your Nix config, or set it to `null` to edit the file yourself again.
- **No tray icon on Linux:** install `libayatana-appindicator` (and, on GNOME, the AppIndicator
  extension). `lucy app` opens the window without it.
- **macOS or Windows won't open the app:** it isn't signed; see [Installing](install.md).
- **Voice and pictures:** [Voice](voice.md#troubleshooting) and [Pictures](images.md#troubleshooting) have their own troubleshooting.

Still stuck? [Open an issue](https://github.com/AJMerr/lucy/issues/new/choose).
