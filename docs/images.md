# Pictures

Lucy can look at pictures and make them.

- **Seeing:** paste, drop or attach a picture in the main window, drop one on the sprite, or ask
  her to look at a picture file (`read_file` shows PNG, JPEG, GIF and WebP files as pictures).
- **Making:** ask for one ("draw me a lighthouse at dusk") and she writes the prompt and makes
  it, or type `/image <description>` to make one straight away, in any chat, the window or the
  sprite. Pictures are saved to `~/Pictures/Lucy` and shown in the window, in the sprite's
  bubble, and inline in terminals that support it.

Everything below can be set in the app under **Settings → Pictures** (and **Models & keys** for
"Can see pictures"), or in `config.toml`.

## Seeing pictures

Mark the models that can see with **Can see pictures** (`vision = true` in the profile). They
get the pictures themselves.

A model that can't see (e.g. a local text model) can still answer questions about pictures if
you choose a **vision helper**: a model that can see describes each picture once, and the
description goes to your chat model. Without one, she tells you she can't see it.

```toml
[profiles.claude]
provider = "anthropic"
model = "claude-sonnet-5-5"
key_name = "anthropic"
vision = true

[images]
vision_profile = "claude"     # describes pictures for models that can't see
```

Pictures you send are kept with the chat in `~/.local/share/lucy/images/`.

## Making pictures

Choose what makes them with `[images] generator`:

| Generator | Cost | Asks first? |
| --- | --- | --- |
| An image model profile (OpenRouter's, for example) | per picture | yes: Allow / Always / Deny |
| `comfyui`: ComfyUI on your own GPU | free, private | no |

`/image` never asks, since you typed it. Pictures she makes are saved to `[images] folder`
(default `~/Pictures/Lucy`), named by date and the first words of the prompt.

### OpenRouter

1. Add a profile for an image model (Settings → Models & keys, or `config.toml`), using your
   OpenRouter key:

   ```toml
   [profiles.pics]
   provider = "openai_compat"
   base_url = "https://openrouter.ai/api/v1"
   model = "google/gemini-2.5-flash-image"   # any OpenRouter model that outputs images
   key_name = "openrouter"
   ```

2. Set it as the generator: Settings → Pictures → Make pictures with → `pics`, or

   ```toml
   [images]
   generator = "pics"
   ```

Image models that output only pictures (no text) work too.

### ComfyUI on your own GPU

[ComfyUI](https://github.com/comfyanonymous/ComfyUI) runs Stable Diffusion-style models
locally. Lucy only needs its address; you run ComfyUI yourself.

#### Running ComfyUI on NixOS

This uses `uv` for ComfyUI's Python and needs `programs.nix-ld.enable = true;` in your NixOS
config, so the Python packages' prebuilt libraries load.

```sh
git clone https://github.com/comfyanonymous/ComfyUI ~/ComfyUI
cd ~/ComfyUI
uv venv --python 3.12
# PyTorch with CUDA 12.8, which RTX 50-series cards need:
uv pip install torch torchvision torchaudio --index-url https://download.pytorch.org/whl/cu128
uv pip install -r requirements.txt
```

Start it (the NVIDIA driver's libraries live in `/run/opengl-driver/lib` on NixOS):

```sh
cd ~/ComfyUI
LD_LIBRARY_PATH=/run/opengl-driver/lib uv run python main.py --listen 127.0.0.1 --port 8188
```

Open <http://127.0.0.1:8188> to check it's up.

#### A model that fits in 12 GB

Lucy's built-in workflow is a standard text-to-image graph for a single checkpoint file, sized
for SDXL (1024×1024 by default). SDXL models use about 7–8 GB of VRAM there, so they fit a 12 GB
card. Download one into `~/ComfyUI/models/checkpoints/`, for example the SDXL base model:

```sh
cd ~/ComfyUI/models/checkpoints
curl -LO https://huggingface.co/stabilityai/stable-diffusion-xl-base-1.0/resolve/main/sd_xl_base_1.0.safetensors
```

Fine-tuned SDXL checkpoints (from Hugging Face or Civitai) work the same way. Larger models
like FLUX need their own workflow (below).

#### Pointing Lucy at it

Settings → Pictures → Make pictures with → **ComfyUI on this computer**, or:

```toml
[images]
generator = "comfyui"

[images.comfyui]
url = "http://127.0.0.1:8188"              # the default
# checkpoint = "sd_xl_base_1.0.safetensors" # default: the first checkpoint ComfyUI has
# workflow = "~/ComfyUI/lucy-flux.json"     # default: Lucy's built-in text-to-image workflow
```

#### Your own workflows

Any workflow can be used: build it in ComfyUI, then **Workflow → Export (API)** and set
`[images.comfyui] workflow` to the file. Put these placeholders in its fields and Lucy fills
them for each picture:

| Placeholder | Filled with |
| --- | --- |
| `{{prompt}}` | what to draw (she writes it, or your `/image` text) |
| `{{negative}}` | what to keep out (empty unless she sets one) |
| `{{width}}`, `{{height}}` | the size, as numbers (1024 unless she asks otherwise) |
| `{{seed}}` | a new random seed, as a number |
| `{{checkpoint}}` | `[images.comfyui] checkpoint`, or ComfyUI's first checkpoint |

A placeholder can be a field's whole value (`"{{width}}"` becomes the number) or part of a
longer prompt (`"{{prompt}}, film grain, 35mm"`). The workflow needs a **Save Image** node;
whatever it saves is what Lucy shows.

## Where pictures show up

- **Main window:** pictures you send show in your message; pictures she makes show under the
  step that made them.
- **Sprite:** drop a picture on her to ask about it. Pictures she makes appear in her bubble;
  click one to open the chat.
- **Terminal:** Ghostty, kitty and WezTerm draw pictures she makes inline (PNG); other terminals,
  and tmux, show where the picture was saved.

## Troubleshooting

- **"nothing is set up to make pictures yet":** choose a generator in Settings → Pictures.
- **ComfyUI "could not connect":** ComfyUI isn't running, or is on another address; check
  <http://127.0.0.1:8188>.
- **"ComfyUI has no checkpoints":** put a model file in `ComfyUI/models/checkpoints/`.
- **"Value not in list: ckpt_name":** `[images.comfyui] checkpoint` names a file ComfyUI doesn't
  have; check the spelling or leave it empty.
- **CUDA out of memory:** use a smaller model or size, or start ComfyUI with `--lowvram`.
- **OpenRouter says the model doesn't support image output:** pick a model whose output
  includes images on openrouter.ai.
- **She says she can't see a picture:** mark the chat's model "Can see pictures" if it can, or
  choose a vision helper.
