//! Making pictures. Every backend implements [`ImageGenerator`]: OpenRouter's image models
//! (through its Image API, `POST /api/v1/images`), and ComfyUI running a workflow file with
//! placeholders filled in.

use std::hash::{BuildHasher, Hasher};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use lucy_protocol::Image;
use serde_json::{Value, json};

use crate::ProviderError;
use crate::http::{check_status, error_message, send_error};
use crate::openai_compat::OpenAiCompatConfig;

/// A text-to-image workflow in ComfyUI's API format, used when none is configured.
pub const DEFAULT_WORKFLOW: &str = include_str!("default_workflow.json");

/// How long a ComfyUI run may take, including loading the model the first time.
const COMFYUI_TIMEOUT: Duration = Duration::from_secs(15 * 60);

/// Aspect ratios OpenRouter's image models generally accept (a model refuses ones it doesn't).
const ASPECT_RATIOS: &[(&str, f64)] = &[
    ("1:1", 1.0),
    ("2:3", 2.0 / 3.0),
    ("3:2", 1.5),
    ("3:4", 0.75),
    ("4:3", 4.0 / 3.0),
    ("4:5", 0.8),
    ("5:4", 1.25),
    ("9:16", 9.0 / 16.0),
    ("16:9", 16.0 / 9.0),
];

#[derive(Debug, Clone, PartialEq)]
pub struct ImageRequest {
    /// What to draw.
    pub prompt: String,
    /// What to keep out of it.
    pub negative: Option<String>,
    pub width: u32,
    pub height: u32,
}

#[async_trait]
pub trait ImageGenerator: Send + Sync {
    async fn generate(&self, request: &ImageRequest) -> Result<Vec<Image>, ProviderError>;
}

// --- OpenRouter ---------------------------------------------------------------------------

/// An OpenRouter image model, through its Image API (`POST /api/v1/images`).
pub struct OpenRouterImages {
    config: OpenAiCompatConfig,
    client: reqwest::Client,
}

impl OpenRouterImages {
    pub fn new(config: OpenAiCompatConfig) -> Self {
        Self {
            config,
            client: reqwest::Client::new(),
        }
    }
}

#[async_trait]
impl ImageGenerator for OpenRouterImages {
    async fn generate(&self, request: &ImageRequest) -> Result<Vec<Image>, ProviderError> {
        // The API root (`…/api/v1`), or the Image API's own address.
        let base = self.config.base_url.trim_end_matches('/');
        let url = match base.ends_with("/images") {
            true => base.to_owned(),
            false => format!("{base}/images"),
        };
        let mut prompt = request.prompt.clone();
        // The Image API has no negative prompt.
        if let Some(negative) = request.negative.as_deref().filter(|n| !n.is_empty()) {
            prompt += &format!("\n\nAvoid: {negative}");
        }
        let mut body = json!({"model": self.config.model, "prompt": prompt});
        // Square is every model's default; only a different shape is asked for.
        let ratio = aspect_ratio(request.width, request.height);
        if ratio != "1:1" {
            body["aspect_ratio"] = json!(ratio);
        }
        let mut http = self.client.post(&url).json(&body);
        if let Some(key) = &self.config.api_key {
            http = http.bearer_auth(key);
        }
        let response = http.send().await.map_err(|err| send_error(&url, err))?;
        let reply: Value = check_status(response)
            .await?
            .json()
            .await
            .map_err(|err| ProviderError::Protocol(format!("bad reply: {err}")))?;
        if let Some(message) = error_message(&reply) {
            return Err(ProviderError::Unavailable(message));
        }
        let pictures: Vec<Image> = reply["data"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|picture| {
                Some(Image {
                    media_type: picture["media_type"]
                        .as_str()
                        .unwrap_or("image/png")
                        .to_owned(),
                    data: picture["b64_json"].as_str()?.to_owned(),
                })
            })
            .collect();
        if pictures.is_empty() {
            return Err(ProviderError::Refused(
                "the model sent back no picture".into(),
            ));
        }
        Ok(pictures)
    }
}

/// The supported aspect ratio nearest `width`:`height`.
fn aspect_ratio(width: u32, height: u32) -> &'static str {
    let ratio = f64::from(width.max(1)) / f64::from(height.max(1));
    ASPECT_RATIOS
        .iter()
        .min_by(|a, b| (a.1 - ratio).abs().total_cmp(&(b.1 - ratio).abs()))
        .map_or("1:1", |(name, _)| name)
}

// --- ComfyUI ------------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct ComfyUiConfig {
    /// Where ComfyUI listens, e.g. `http://127.0.0.1:8188`.
    pub url: String,
    /// The workflow in ComfyUI's API format ("Export (API)"), with placeholders.
    pub workflow: Value,
    /// Fills `{{checkpoint}}`; ComfyUI's first checkpoint when unset.
    pub checkpoint: Option<String>,
    pub poll_every: Duration,
}

/// A local ComfyUI server. Placeholders in the workflow's strings are filled per picture:
/// `{{prompt}}`, `{{negative}}` and `{{checkpoint}}` as text, and `{{width}}`, `{{height}}` and
/// `{{seed}}` (when a whole value) as numbers.
pub struct ComfyUi {
    config: ComfyUiConfig,
    client: reqwest::Client,
}

impl ComfyUi {
    pub fn new(config: ComfyUiConfig) -> Self {
        Self {
            config,
            client: reqwest::Client::new(),
        }
    }

    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.config.url.trim_end_matches('/'))
    }

    async fn get(
        &self,
        path: &str,
        query: &[(&str, &str)],
    ) -> Result<reqwest::Response, ProviderError> {
        let mut url = reqwest::Url::parse(&self.url(path))
            .map_err(|err| ProviderError::Request(format!("bad ComfyUI address: {err}")))?;
        if !query.is_empty() {
            url.query_pairs_mut().extend_pairs(query);
        }
        let url = url.to_string();
        let response = self
            .client
            .get(&url)
            .send()
            .await
            .map_err(|err| send_error(&url, err))?;
        check_status(response).await
    }

    async fn get_json(&self, path: &str) -> Result<Value, ProviderError> {
        self.get(path, &[])
            .await?
            .json()
            .await
            .map_err(|err| ProviderError::Protocol(format!("bad reply from ComfyUI: {err}")))
    }

    /// The configured checkpoint, or the first one ComfyUI has.
    async fn checkpoint(&self) -> Result<String, ProviderError> {
        if let Some(name) = &self.config.checkpoint {
            return Ok(name.clone());
        }
        let info = self.get_json("/object_info/CheckpointLoaderSimple").await?;
        info["CheckpointLoaderSimple"]["input"]["required"]["ckpt_name"][0][0]
            .as_str()
            .map(str::to_owned)
            .ok_or_else(|| {
                ProviderError::Request(
                    "ComfyUI has no checkpoints; put a model in ComfyUI's models/checkpoints \
                     folder"
                        .into(),
                )
            })
    }

    /// Queue the workflow; returns its prompt id.
    async fn queue(&self, graph: Value) -> Result<String, ProviderError> {
        let url = self.url("/prompt");
        let response = self
            .client
            .post(&url)
            .json(&json!({"prompt": graph, "client_id": "lucy"}))
            .send()
            .await
            .map_err(|err| send_error(&url, err))?;
        if response.status() == reqwest::StatusCode::BAD_REQUEST {
            let body: Value = response.json().await.unwrap_or_default();
            return Err(ProviderError::Request(rejection(&body)));
        }
        let body: Value = check_status(response)
            .await?
            .json()
            .await
            .map_err(|err| ProviderError::Protocol(format!("bad reply from ComfyUI: {err}")))?;
        body["prompt_id"]
            .as_str()
            .map(str::to_owned)
            .ok_or_else(|| ProviderError::Protocol(format!("ComfyUI sent no prompt id: {body}")))
    }

    /// Wait for a run to finish; returns its history entry.
    async fn wait(&self, id: &str) -> Result<Value, ProviderError> {
        let started = Instant::now();
        loop {
            let mut history = self.get_json(&format!("/history/{id}")).await?;
            let entry = history[id].take();
            match entry["status"]["status_str"].as_str() {
                Some("success") => return Ok(entry),
                Some("error") => return Err(ProviderError::Unavailable(run_error(&entry))),
                _ => {}
            }
            if started.elapsed() > COMFYUI_TIMEOUT {
                return Err(ProviderError::Unavailable(format!(
                    "ComfyUI didn't finish within {} minutes",
                    COMFYUI_TIMEOUT.as_secs() / 60
                )));
            }
            tokio::time::sleep(self.config.poll_every).await;
        }
    }

    async fn download(&self, file: &Value) -> Result<Image, ProviderError> {
        let field = |key: &str| file[key].as_str().unwrap_or_default();
        let query = ["filename", "subfolder", "type"].map(|key| (key, field(key)));
        let response = self.get("/view", &query).await?;
        let media_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .filter(|v| v.starts_with("image/"))
            .unwrap_or("image/png")
            .to_owned();
        let bytes = response
            .bytes()
            .await
            .map_err(|err| ProviderError::Unavailable(format!("couldn't download: {err}")))?;
        Ok(Image {
            media_type,
            data: STANDARD.encode(bytes),
        })
    }
}

#[async_trait]
impl ImageGenerator for ComfyUi {
    async fn generate(&self, request: &ImageRequest) -> Result<Vec<Image>, ProviderError> {
        let mut graph = self.config.workflow.clone();
        let checkpoint = if graph.to_string().contains("{{checkpoint}}") {
            self.checkpoint().await?
        } else {
            String::new()
        };
        fill(
            &mut graph,
            &Fill {
                prompt: &request.prompt,
                negative: request.negative.as_deref().unwrap_or_default(),
                checkpoint: &checkpoint,
                width: request.width,
                height: request.height,
                seed: seed(),
            },
        );
        let id = self.queue(graph).await?;
        let entry = self.wait(&id).await?;
        let files: Vec<&Value> = entry["outputs"]
            .as_object()
            .into_iter()
            .flat_map(|outputs| outputs.values())
            .filter_map(|output| output["images"].as_array())
            .flatten()
            .filter(|file| file["type"] == "output")
            .collect();
        if files.is_empty() {
            return Err(ProviderError::Request(
                "the workflow finished without saving a picture; it needs a Save Image node".into(),
            ));
        }
        let mut pictures = Vec::new();
        for file in files {
            pictures.push(self.download(file).await?);
        }
        Ok(pictures)
    }
}

struct Fill<'a> {
    prompt: &'a str,
    negative: &'a str,
    checkpoint: &'a str,
    width: u32,
    height: u32,
    seed: u64,
}

/// Fill placeholders throughout a workflow.
fn fill(value: &mut Value, with: &Fill) {
    match value {
        Value::String(text) => {
            *value = match text.as_str() {
                "{{width}}" => json!(with.width),
                "{{height}}" => json!(with.height),
                "{{seed}}" => json!(with.seed),
                _ => Value::String(
                    text.replace("{{prompt}}", with.prompt)
                        .replace("{{negative}}", with.negative)
                        .replace("{{checkpoint}}", with.checkpoint)
                        .replace("{{width}}", &with.width.to_string())
                        .replace("{{height}}", &with.height.to_string())
                        .replace("{{seed}}", &with.seed.to_string()),
                ),
            }
        }
        Value::Array(items) => items.iter_mut().for_each(|item| fill(item, with)),
        Value::Object(fields) => fields.values_mut().for_each(|field| fill(field, with)),
        _ => {}
    }
}

/// A fresh seed for each picture (kept within what JavaScript numbers hold exactly).
fn seed() -> u64 {
    let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
    hasher.write_u128(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos(),
    );
    hasher.finish() >> 11
}

/// Why ComfyUI refused a workflow: its message, then each node's errors.
fn rejection(body: &Value) -> String {
    let mut text = error_message(body).unwrap_or_else(|| "ComfyUI rejected the workflow".into());
    for (node, errors) in body["node_errors"].as_object().into_iter().flatten() {
        let class = errors["class_type"].as_str().unwrap_or("node");
        for error in errors["errors"].as_array().into_iter().flatten() {
            let message = error["message"].as_str().unwrap_or_default();
            let details = error["details"].as_str().unwrap_or_default();
            text += &format!("\n{class} (node {node}): {message}: {details}");
        }
    }
    text
}

/// Why a run failed, from its status messages.
fn run_error(entry: &Value) -> String {
    entry["status"]["messages"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|m| m[0] == "execution_error")
        .map(|m| {
            format!(
                "ComfyUI failed in {}: {}",
                m[1]["node_type"].as_str().unwrap_or("a node"),
                m[1]["exception_message"]
                    .as_str()
                    .unwrap_or("unknown error")
                    .trim()
            )
        })
        .next()
        .unwrap_or_else(|| "ComfyUI's run failed".into())
}
