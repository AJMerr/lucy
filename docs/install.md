# Installing

Lucy comes in two parts, and the installers give you both:

- **`lucy`**, the command: chat in a terminal, set things up, and run Lucy's **core**, the
  background program that talks to models, keeps your chats and runs tools. Everything else
  connects to the core, and `lucy` starts it when it's needed.
- **Lucy's app** (`lucy-desktop`): her sprite on your desktop, a main window for your chats and
  every setting, and a tray icon. No terminal needed.

## macOS

```sh
brew install --cask ajmerr/lucy/lucy     # the app, with `lucy` inside
brew install ajmerr/lucy/lucy            # or just the `lucy` command
```

Or download the `.dmg` from the [latest release](https://github.com/AJMerr/lucy/releases/latest)
(Apple silicon). `lucy` is inside the app, at `Lucy.app/Contents/MacOS/lucy`; the cask links it
onto your `PATH`.

**"Lucy can't be opened because Apple cannot check it for malicious software."** Lucy isn't
signed with a paid Apple developer certificate, so Gatekeeper warns the first time. To open it
anyway: right-click (or Control-click) Lucy in Applications, choose **Open**, then **Open**
again. On recent macOS versions, if there's no Open button, go to System Settings → Privacy &
Security, scroll down to the message about Lucy and click **Open Anyway**. You only do this once.
(With Homebrew, `brew install --cask --no-quarantine ajmerr/lucy/lucy` skips the prompt.)

## Windows

Download the `.msi` from the [latest release](https://github.com/AJMerr/lucy/releases/latest)
and run it. `lucy.exe` is installed next to `lucy-desktop.exe` (in `C:\Program Files\Lucy\` by
default); add that folder to your `PATH` to use `lucy` in a terminal.

**"Windows protected your PC."** Lucy isn't signed with a paid code-signing certificate, so
SmartScreen warns about an unrecognised app. Click **More info**, then **Run anyway**.

## Linux

- **Arch:** `yay -S lucy-bin` (or any AUR helper).
- **Debian, Ubuntu:** the `.deb` from the [latest release](https://github.com/AJMerr/lucy/releases/latest),
  then `sudo apt install ./Lucy_*_amd64.deb`.
- **Fedora, openSUSE:** the `.rpm`, then `sudo dnf install ./Lucy-*.x86_64.rpm`.
- **Anything else:** the `.AppImage`; make it executable and run it.
- **Nix:** see [Nix and home-manager](nix.md).

`lucy` lands in `/usr/bin`. Keys go in your desktop's keyring, so you need a Secret Service
provider (GNOME Keyring, KWallet, KeePassXC…); most desktops have one. The tray icon needs
AppIndicator (`libayatana-appindicator`, and on GNOME the AppIndicator extension).

## From source

You need a Rust toolchain:

```sh
cargo install --locked --git https://github.com/AJMerr/lucy lucy-cli    # installs `lucy`
```

Make sure `~/.cargo/bin` is on your `PATH`. Building the app is covered in
[CONTRIBUTING.md](https://github.com/AJMerr/lucy/blob/main/CONTRIBUTING.md).

## A model to talk to

Lucy needs a model: one on your own computer, or a hosted one with an API key.

- **On your computer:** install [Ollama](https://ollama.com) and pull a model that can use
  tools, e.g. `ollama pull qwen3:14b` (about 9 GB; it wants a GPU with 12 GB or more, or a
  recent Mac). Smaller ones like `qwen3:8b` work too, with less judgement. On NixOS, set
  `services.ollama.enable = true;` (with `package = pkgs.ollama-cuda;` for NVIDIA GPUs).
- **Hosted:** an API key from [OpenRouter](https://openrouter.ai) (hundreds of models with one
  key), [Anthropic](https://console.anthropic.com), [OpenAI](https://platform.openai.com) or
  Qwen Cloud.

Then carry on with [the first five minutes](getting-started.md).

## Updating

Install the new version the same way. If Lucy's core was running, it carries on with the old
version until it restarts: `lucy` restarts it by itself when it's idle, and the app offers a
**Restart it** button. Or run `lucy service restart` (`lucy core stop` if you don't use the
service).

Your config, chats, memory and keys are kept. From 1.0, updates within 1.x never break your
config.toml or the `lucy` command; see [what stays stable](stability.md).

## Uninstalling

Remove the app or package the way you installed it. Then, if you want to remove everything:

- `lucy service uninstall` first, if you set up the login service.
- Delete the config (`~/.config/lucy/`) and data (`~/.local/share/lucy/`: chats, memory,
  voice models). On macOS both are in `~/Library/Application Support/lucy/`; on Windows in
  `%APPDATA%\lucy\`.
- Remove the keys from your keyring (`lucy auth remove <name>` for each, before uninstalling).
- Pictures she made are in `~/Pictures/Lucy/`.
