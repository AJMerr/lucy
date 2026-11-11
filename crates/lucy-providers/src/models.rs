//! What a provider offers: its models, and each one's context size and whether it can see,
//! where the provider says. Ollama answers through its own API (`/api/tags`, `/api/show`);
//! other OpenAI-compatible servers through `/models` (OpenRouter's includes context sizes and
//! input types); Anthropic through `/v1/models`.

use serde_json::{Value, json};

use crate::ProviderError;
use crate::http::{check_status, send_error};

/// Which API a profile speaks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    OpenAiCompat,
    Anthropic,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ModelInfo {
    /// What to put in a profile's `model`.
    pub id: String,
    /// A friendlier name, when the provider has one.
    pub name: Option<String>,
    /// How many tokens fit in its context window.
    pub context_tokens: Option<u32>,
    /// It takes pictures.
    pub vision: Option<bool>,
}

const ANTHROPIC: &str = "https://api.anthropic.com";
const ANTHROPIC_VERSION: &str = "2023-06-01";

/// Every model `base_url` offers, sorted by ID.
pub async fn list_models(
    kind: Kind,
    base_url: &str,
    api_key: Option<&str>,
) -> Result<Vec<ModelInfo>, ProviderError> {
    let client = reqwest::Client::new();
    let mut models = match kind {
        Kind::Anthropic => anthropic_models(&client, base_url, api_key).await?,
        Kind::OpenAiCompat => match ollama_tags(&client, base_url).await {
            Some(models) => models,
            None => openai_models(&client, base_url, api_key).await?,
        },
    };
    models.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(models)
}

/// One model's details: from Ollama's `/api/show`, or the provider's list.
pub async fn describe_model(
    kind: Kind,
    base_url: &str,
    api_key: Option<&str>,
    model: &str,
) -> Result<ModelInfo, ProviderError> {
    if kind == Kind::OpenAiCompat
        && let Some(info) = ollama_show(&reqwest::Client::new(), base_url, model).await
    {
        return Ok(info);
    }
    let models = list_models(kind, base_url, api_key).await?;
    Ok(models
        .into_iter()
        .find(|m| m.id == model)
        .unwrap_or_else(|| ModelInfo {
            id: model.into(),
            name: None,
            context_tokens: None,
            vision: None,
        }))
}

async fn get_json(request: reqwest::RequestBuilder, url: &str) -> Result<Value, ProviderError> {
    let response = request.send().await.map_err(|err| send_error(url, err))?;
    check_status(response)
        .await?
        .json()
        .await
        .map_err(|err| ProviderError::Protocol(format!("bad model list: {err}")))
}

/// Ollama's own API sits beside its OpenAI-compatible one (`…/v1`).
fn ollama_root(base_url: &str) -> &str {
    let base = base_url.trim_end_matches('/');
    base.strip_suffix("/v1").unwrap_or(base)
}

async fn ollama_tags(client: &reqwest::Client, base_url: &str) -> Option<Vec<ModelInfo>> {
    let url = format!("{}/api/tags", ollama_root(base_url));
    let tags: Value = get_json(client.get(&url), &url).await.ok()?;
    let models = tags["models"].as_array()?;
    Some(
        models
            .iter()
            .filter_map(|m| m["name"].as_str())
            .map(|name| ModelInfo {
                id: name.to_owned(),
                name: None,
                context_tokens: None,
                vision: None,
            })
            .collect(),
    )
}

async fn ollama_show(client: &reqwest::Client, base_url: &str, model: &str) -> Option<ModelInfo> {
    let url = format!("{}/api/show", ollama_root(base_url));
    let shown: Value = get_json(client.post(&url).json(&json!({"model": model})), &url)
        .await
        .ok()?;
    let context = shown["model_info"]
        .as_object()?
        .iter()
        .find(|(key, _)| key.ends_with(".context_length"))
        .and_then(|(_, v)| v.as_u64())
        .map(|n| n.min(u64::from(u32::MAX)) as u32);
    let vision = shown["capabilities"]
        .as_array()
        .map(|caps| caps.iter().any(|c| c == "vision"));
    Some(ModelInfo {
        id: model.to_owned(),
        name: None,
        context_tokens: context,
        vision,
    })
}

async fn openai_models(
    client: &reqwest::Client,
    base_url: &str,
    api_key: Option<&str>,
) -> Result<Vec<ModelInfo>, ProviderError> {
    let url = format!("{}/models", base_url.trim_end_matches('/'));
    let mut request = client.get(&url);
    if let Some(key) = api_key {
        request = request.bearer_auth(key);
    }
    let list = get_json(request, &url).await?;
    Ok(list["data"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|m| {
            Some(ModelInfo {
                id: m["id"].as_str()?.to_owned(),
                name: m["name"].as_str().map(str::to_owned),
                context_tokens: m["context_length"]
                    .as_u64()
                    .map(|n| n.min(u64::from(u32::MAX)) as u32),
                vision: m["architecture"]["input_modalities"]
                    .as_array()
                    .map(|inputs| inputs.iter().any(|i| i == "image")),
            })
        })
        .collect())
}

async fn anthropic_models(
    client: &reqwest::Client,
    base_url: &str,
    api_key: Option<&str>,
) -> Result<Vec<ModelInfo>, ProviderError> {
    let base = if base_url.is_empty() {
        ANTHROPIC
    } else {
        base_url
    };
    let url = format!("{}/v1/models?limit=1000", base.trim_end_matches('/'));
    let request = client
        .get(&url)
        .header("x-api-key", api_key.unwrap_or_default())
        .header("anthropic-version", ANTHROPIC_VERSION);
    let list = get_json(request, &url).await?;
    Ok(list["data"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|m| {
            Some(ModelInfo {
                id: m["id"].as_str()?.to_owned(),
                name: m["display_name"].as_str().map(str::to_owned),
                context_tokens: m["max_input_tokens"]
                    .as_u64()
                    .map(|n| n.min(u64::from(u32::MAX)) as u32),
                // Every current Claude model takes pictures.
                vision: Some(true),
            })
        })
        .collect())
}
