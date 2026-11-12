//! Making pictures, for the `generate_image` tool and `/image`. The configured generator
//! (`[images] generator`) makes them; each is saved to your pictures folder and kept with the
//! chat like a picture you sent. ComfyUI shares the GPU with a local Ollama badly (a 12 GB
//! card can't hold a chat model and an image model at once), so before each picture the
//! chat's Ollama unloads its models (`[images.comfyui] unload_ollama`).

use std::path::PathBuf;
use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use lucy_protocol::rpc::ImageData;
use lucy_providers::generate::{
    ComfyUi, ComfyUiConfig, DEFAULT_WORKFLOW, ImageGenerator, ImageRequest,
};
use serde_json::Value;

use crate::config::{COMFYUI, Config, Profile, ProviderKind};
use crate::mcp::ToolOutput;
use crate::provider::build_generator;
use crate::secrets::{SecretStore, Secrets};

/// The tool she calls to make a picture.
pub const TOOL: &str = "generate_image";

pub const DEFAULT_SIZE: u32 = 1024;
/// The largest side she may ask for.
const MAX_SIDE: u32 = 2048;

/// The configured generator, ready to make pictures.
pub struct Generator {
    backend: Box<dyn ImageGenerator>,
    /// Hosted, so each picture costs money: the tool asks first.
    pub hosted: bool,
    /// What it's called in approval prompts: the profile, or "ComfyUI".
    pub label: String,
    /// Your pictures folder.
    folder: PathBuf,
    /// The chat's Ollama, whose models are unloaded before each picture.
    ollama: Option<url::Url>,
}

/// A picture made and saved.
pub struct Made {
    /// Where it was saved in your pictures folder.
    pub path: PathBuf,
    pub data: ImageData,
}

impl Generator {
    /// The generator `config` names; `Ok(None)` when none is set.
    pub fn from_config<S, E>(
        config: &Config,
        secrets: &Secrets<S, E>,
    ) -> Result<Option<Self>, String>
    where
        S: SecretStore,
        E: Fn(&str) -> Option<String>,
    {
        let Some(name) = &config.images.generator else {
            return Ok(None);
        };
        let (backend, hosted, label): (Box<dyn ImageGenerator>, bool, String) = if name == COMFYUI {
            let settings = &config.images.comfyui;
            let workflow = match &settings.workflow {
                Some(path) => {
                    let path = crate::server::expand_home(path);
                    std::fs::read_to_string(&path).map_err(|err| {
                        format!(
                            "couldn't read the ComfyUI workflow {}: {err}",
                            path.display()
                        )
                    })?
                }
                None => DEFAULT_WORKFLOW.to_owned(),
            };
            let workflow: Value = serde_json::from_str(&workflow).map_err(|err| {
                format!(
                    "the ComfyUI workflow isn't valid JSON ({err}); export it with \"Export (API)\""
                )
            })?;
            let backend = ComfyUi::new(ComfyUiConfig {
                url: settings.url.clone(),
                workflow,
                checkpoint: settings.checkpoint.clone(),
                poll_every: Duration::from_millis(500),
            });
            (Box::new(backend), false, "ComfyUI".into())
        } else {
            let profile = config
                .profiles
                .get(name)
                .ok_or_else(|| format!("no profile `{name}` to make pictures with"))?;
            let backend = build_generator(name, profile, secrets).map_err(|err| err.to_string())?;
            (backend, true, name.clone())
        };
        let folder = match &config.images.folder {
            Some(folder) => crate::server::expand_home(folder),
            None => default_folder(),
        };
        Ok(Some(Self {
            backend,
            hosted,
            label,
            folder,
            ollama: None,
        }))
    }

    /// For a chat with `chat`'s model: if that's an Ollama on this machine, ComfyUI gets the
    /// GPU from it before each picture (`[images.comfyui] unload_ollama`).
    #[must_use]
    pub fn beside(mut self, config: &Config, chat: &Profile) -> Self {
        let wanted = config.images.generator.as_deref() == Some(COMFYUI)
            && config.images.comfyui.unload_ollama
            && chat.provider == ProviderKind::OpenAiCompat
            && chat.is_local();
        self.ollama = chat
            .base_url
            .as_deref()
            .filter(|_| wanted)
            .and_then(|base| url::Url::parse(base).ok())
            .and_then(|url| url.join("/").ok());
        self
    }

    /// Make pictures for `request` and save them.
    pub async fn make(&self, request: &ImageRequest) -> Result<Vec<Made>, String> {
        let request = ImageRequest {
            width: request.width.clamp(64, MAX_SIDE),
            height: request.height.clamp(64, MAX_SIDE),
            ..request.clone()
        };
        if let Some(ollama) = &self.ollama {
            unload_ollama(ollama).await;
        }
        let pictures = self
            .backend
            .generate(&request)
            .await
            .map_err(|err| err.to_string())?;
        let mut made = Vec::new();
        for picture in pictures {
            let data = ImageData {
                media_type: picture.media_type,
                data: picture.data,
            };
            let path = self.keep(&request.prompt, &data)?;
            made.push(Made { path, data });
        }
        Ok(made)
    }

    /// Save a copy in the pictures folder, named by when it was made and what it shows.
    fn keep(&self, prompt: &str, data: &ImageData) -> Result<PathBuf, String> {
        let bytes = STANDARD
            .decode(data.data.as_bytes())
            .map_err(|err| format!("the picture came back garbled: {err}"))?;
        std::fs::create_dir_all(&self.folder)
            .map_err(|err| format!("couldn't create {}: {err}", self.folder.display()))?;
        let ext = match data.media_type.as_str() {
            "image/jpeg" => "jpg",
            "image/webp" => "webp",
            "image/gif" => "gif",
            _ => "png",
        };
        let stem = format!(
            "{}-{}",
            chrono::Local::now().format("%Y-%m-%d_%H-%M-%S"),
            slug(prompt)
        );
        let mut path = self.folder.join(format!("{stem}.{ext}"));
        let mut n = 2;
        while path.exists() {
            path = self.folder.join(format!("{stem}-{n}.{ext}"));
            n += 1;
        }
        std::fs::write(&path, bytes)
            .map_err(|err| format!("couldn't save {}: {err}", path.display()))?;
        Ok(path)
    }
}

/// Ask the Ollama at `root` to unload every model it has loaded, and wait (briefly) until it
/// has. Anything else at that address (LM Studio, llama.cpp) has no `/api/ps` and is left
/// alone; failures only cost speed, so they're logged and the picture goes ahead.
async fn unload_ollama(root: &url::Url) {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap_or_default();
    let loaded = |client: reqwest::Client| async move {
        let response = client.get(root.join("api/ps").ok()?).send().await.ok()?;
        let ps: Value = response.error_for_status().ok()?.json().await.ok()?;
        Some(
            ps["models"]
                .as_array()?
                .iter()
                .filter_map(|m| m["name"].as_str().map(str::to_owned))
                .collect::<Vec<_>>(),
        )
    };
    let Some(models) = loaded(client.clone()).await else {
        return;
    };
    let Ok(generate) = root.join("api/generate") else {
        return;
    };
    for model in &models {
        let unload = serde_json::json!({"model": model, "keep_alive": 0});
        if let Err(err) = client.post(generate.clone()).json(&unload).send().await {
            eprintln!("couldn't unload {model} from Ollama before the picture: {err}");
        }
    }
    // Ollama frees the memory a moment after answering.
    for _ in 0..10 {
        match loaded(client.clone()).await {
            Some(still) if still.iter().any(|m| models.contains(m)) => {
                tokio::time::sleep(Duration::from_millis(300)).await;
            }
            _ => return,
        }
    }
}

/// `~/Pictures/Lucy`.
pub fn default_folder() -> PathBuf {
    dirs::picture_dir()
        .or_else(|| dirs::home_dir().map(|home| home.join("Pictures")))
        .unwrap_or_default()
        .join("Lucy")
}

/// The first few words of a prompt, for a file name.
fn slug(prompt: &str) -> String {
    let words: Vec<String> = prompt
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .take(6)
        .map(str::to_lowercase)
        .collect();
    if words.is_empty() {
        "picture".into()
    } else {
        words.join("-").chars().take(60).collect()
    }
}

/// What the model is told `generate_image` does.
pub const DESCRIPTION: &str = "Make a picture from a description and show it to the user. \
     Write the prompt yourself from what they asked for, in detail: subject, setting, style, \
     composition, lighting and colours. The picture is saved to their pictures folder.";

/// `generate_image`'s arguments, as a JSON Schema.
pub fn input_schema() -> Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "prompt": {"type": "string", "description": "What to draw, in detail."},
            "negative_prompt": {"type": "string", "description": "What to keep out of it."},
            "width": {"type": "integer", "description": "Pixels (default 1024)."},
            "height": {"type": "integer", "description": "Pixels (default 1024)."},
        },
        "required": ["prompt"],
    })
}

/// A `generate_image` call's arguments as a request.
pub fn request(arguments: &Value) -> Result<ImageRequest, String> {
    let prompt = arguments["prompt"]
        .as_str()
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .ok_or("give a `prompt` describing the picture")?;
    let side = |key: &str| {
        arguments[key]
            .as_u64()
            .map_or(DEFAULT_SIZE, |n| n.min(u64::from(MAX_SIDE)) as u32)
    };
    Ok(ImageRequest {
        prompt: prompt.to_owned(),
        negative: arguments["negative_prompt"].as_str().map(str::to_owned),
        width: side("width"),
        height: side("height"),
    })
}

impl Generator {
    /// Make pictures for a `generate_image` call, as its result.
    pub async fn run(&self, request: &ImageRequest) -> ToolOutput {
        match self.make(request).await {
            Ok(made) => {
                let paths: Vec<String> =
                    made.iter().map(|m| m.path.display().to_string()).collect();
                ToolOutput {
                    text: format!(
                        "Made the picture; the user can see it. Saved to {}.",
                        paths.join(", ")
                    ),
                    is_error: false,
                    images: made.into_iter().map(|m| m.data).collect(),
                    made: true,
                }
            }
            Err(err) => ToolOutput::failed(format!("Couldn't make the picture: {err}")),
        }
    }
}
