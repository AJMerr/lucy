# Voice

Talk to Lucy and hear her answer, in her desktop app (the sprite and the main window). The
terminal stays text.

## Turning it on

Settings → **Voice** → **Talk to Lucy**, then **Save**. The first time, the app downloads what
voice needs into Lucy's data folder (`~/.local/share/lucy/voice/`), showing progress:

- the speech engine ([sherpa-onnx](https://github.com/k2-fsa/sherpa-onnx), about 25–40 MB);
- the model that hears you (**Hearing**: Whisper tiny 116 MB, base 207 MB, small 640 MB, or
  turbo 560 MB);
- her voice (Kokoro about 320 MB, or a Piper voice about 65 MB), unless you use a hosted one;
- a small speech detector (to know when you've stopped), and the wake word model if it's on.

Nothing is bundled with Lucy, and building Lucy needs none of it. Hearing you always happens on
your computer.

On macOS, the first time you talk the system asks whether Lucy may use the microphone.

## Talking

- **Hold** the 🎙 button (in the main window's message box, or by the sprite's input box) and
  talk; let go when you're done.
- **Click** it once instead and just talk: she stops listening when you pause.
- On Wayland (Hyprland, Sway, …), bind `lucy sprite talk` to a key; she listens until you pause:

  ```
  bind = SUPER, V, exec, lucy sprite talk
  ```

What you say is sent as a message in the chat you said it in, and her reply is read aloud a
sentence at a time as it arrives. Code blocks, tables and link addresses stay in the chat
("I've put the code in the chat"). Talking to her again, or Stop, cuts her off.

**Read typed replies aloud** reads her replies to what you type, too.

## Her voice

| Voice | Where it runs | Notes |
| --- | --- | --- |
| **Kokoro** (default) | your computer | natural; 11 speakers (`af_sarah` by default) |
| **Piper** | your computer | lighter and faster, a little more robotic |
| **OpenAI** | hosted | needs an OpenAI API key (saved to the keyring from Settings → Voice); voices like `nova` |
| **ElevenLabs** | hosted | needs an ElevenLabs API key and a voice ID |

**Preview** says hello in the chosen voice. Hosted voices send the text of her replies to that
service, and cost what it charges.

## "Hey Lucy"

Turn on **Listen for "Hey Lucy"** and, while the app runs, saying "Hey Lucy" makes the sprite
listen until you pause, as if you'd clicked the mic. This keeps the microphone open (on this
computer only: nothing is sent anywhere until you've finished speaking, and then only as text to
your chat). The wake word is a small keyword-spotting model that runs on your CPU.

## Settings in config.toml

```toml
[voice]
enabled = true
hearing = "small"          # tiny | base | small | turbo
voice = "kokoro"           # kokoro | piper | openai | elevenlabs
speaker = "af_sarah"       # a Kokoro speaker, Piper voice, OpenAI voice or ElevenLabs voice ID
read_typed = false         # also read replies to typed messages
wake_word = false          # listen for "Hey Lucy"
```

Hosted voices use the keys `openai` and `elevenlabs` in the keyring (`lucy auth set openai`).

## Troubleshooting

- **No 🎙 button:** voice isn't on, or is still downloading (Settings → Voice shows progress and
  any error).
- **"no microphone found" / "no speakers found":** check your system's default input and output
  devices. On Linux, Lucy uses ALSA (PipeWire and PulseAudio provide it).
- **She hears you wrong:** a bigger **Hearing** model is more accurate (and slower on a CPU).
- **The wake word fires by itself, or never:** it's tuned to "Hey Lucy" said clearly; turn it off
  and use push-to-talk if your room is noisy.
- **Linux, the engine won't load:** it needs the C++ runtime (`libstdc++`), which desktop Linux
  has; on NixOS, use the flake's `lucy-desktop` package (it puts it on the library path).
