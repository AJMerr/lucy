//! Embeddings through an OpenAI-compatible `/embeddings` endpoint (Ollama, OpenAI,
//! OpenRouter...), for finding memories by meaning.

use serde_json::{Value, json};

use crate::ProviderError;
use crate::http::{check_status, send_error};
use crate::openai_compat::OpenAiCompatConfig;

pub struct Embedder {
    config: OpenAiCompatConfig,
    client: reqwest::Client,
}

impl Embedder {
    pub fn new(config: OpenAiCompatConfig) -> Self {
        Self {
            config,
            client: reqwest::Client::new(),
        }
    }

    /// The embedding model's name, to tell its vectors apart from another model's.
    pub fn model(&self) -> &str {
        &self.config.model
    }

    /// One vector per text, in order.
    pub async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, ProviderError> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }
        let url = format!("{}/embeddings", self.config.base_url.trim_end_matches('/'));
        let mut http = self
            .client
            .post(&url)
            .json(&json!({"model": self.config.model, "input": texts}));
        if let Some(key) = &self.config.api_key {
            http = http.bearer_auth(key);
        }
        let response = http.send().await.map_err(|err| send_error(&url, err))?;
        let reply: Value = check_status(response)
            .await?
            .json()
            .await
            .map_err(|err| ProviderError::Protocol(format!("bad embeddings reply: {err}")))?;
        let mut vectors: Vec<Option<Vec<f32>>> = vec![None; texts.len()];
        for (n, item) in reply["data"].as_array().into_iter().flatten().enumerate() {
            let index = item["index"].as_u64().map_or(n, |i| i as usize);
            let vector = item["embedding"].as_array().map(|v| {
                v.iter()
                    .filter_map(Value::as_f64)
                    .map(|x| x as f32)
                    .collect()
            });
            if let (Some(slot), Some(vector)) = (vectors.get_mut(index), vector) {
                *slot = Some(vector);
            }
        }
        vectors
            .into_iter()
            .map(|v| v.ok_or_else(|| ProviderError::Protocol("an embedding was missing".into())))
            .collect()
    }
}
