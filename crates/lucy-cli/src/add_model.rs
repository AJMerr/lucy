//! `lucy config add-model`: add a model without knowing its ID or address. Pick a provider,
//! give its key if it needs one, choose from the models it actually offers, try the one you
//! picked, and save it as a profile (with its context size and sight, when the provider says).

use std::io::Write;

use anyhow::{Result, bail};
use lucy_client::Client;
use lucy_core::setup;
use lucy_protocol::rpc::{KeySetParams, ModelsListParams, ModelsTryParams, method};
use lucy_providers::models::ModelInfo;
use serde_json::{Value, json};

use crate::wizard::Prompter;

/// More models than this, and you're asked for part of the name first.
const LONG_LIST: usize = 25;

pub async fn run(
    client: &Client,
    prompter: &mut impl Prompter,
    out: &mut impl Write,
) -> Result<String> {
    let presets = setup::presets();
    let labels: Vec<&str> = presets.iter().map(|p| p.label.as_str()).collect();
    let preset = &presets[prompter.select("Which provider?", &labels, 0)?];
    let base_url = match preset.provider.as_str() {
        "anthropic" => preset.base_url.clone().unwrap_or_default(),
        _ => prompter.input("Address", preset.base_url.as_deref())?,
    };
    let key_name = match &preset.key_name {
        Some(name) => Some(name.clone()),
        None if preset.base_url.is_none() => {
            Some(prompter.input("API key name (leave empty if none)", Some(""))?)
                .filter(|name| !name.is_empty())
        }
        None => None,
    };
    if let Some(name) = &key_name {
        let key = prompter.input(
            &format!("API key for {name} (leave empty if it's already saved)"),
            Some(""),
        )?;
        if !key.trim().is_empty() {
            let _: Value = client
                .call(
                    method::KEYS_SET,
                    KeySetParams {
                        name: name.clone(),
                        value: key.trim().to_owned(),
                    },
                )
                .await?;
        }
    }

    let models: Vec<ModelInfo> = client
        .call(
            method::MODELS_LIST,
            ModelsListParams {
                provider: preset.provider.clone(),
                base_url: Some(base_url.clone()),
                key_name: key_name.clone(),
                key: None,
            },
        )
        .await
        .unwrap_or_default();
    let model = choose(prompter, &models)?;

    let name = prompter.input("Profile name", Some(&profile_name(&model)))?;
    writeln!(out, "Trying {model}…")?;
    let tried: Result<Value, _> = client
        .call(
            method::MODELS_TRY,
            ModelsTryParams {
                provider: preset.provider.clone(),
                base_url: Some(base_url.clone()),
                model: model.clone(),
                key_name: key_name.clone(),
                key: None,
            },
        )
        .await;
    let info = match tried {
        Ok(tried) => {
            writeln!(
                out,
                "It answered: {}",
                tried["reply"].as_str().unwrap_or_default()
            )?;
            serde_json::from_value::<ModelInfo>(tried["info"].clone()).ok()
        }
        Err(err) => {
            writeln!(out, "It didn't answer: {err}")?;
            let choice = prompter.select("Save it anyway?", &["Save anyway", "Cancel"], 1)?;
            if choice == 1 {
                bail!("not saved");
            }
            None
        }
    };
    // What the list said, unless trying it said more.
    let known = models.iter().find(|m| m.id == model);
    let vision = info
        .as_ref()
        .and_then(|i| i.vision)
        .or(known.and_then(|m| m.vision))
        .unwrap_or(false);
    let context = info
        .as_ref()
        .and_then(|i| i.context_tokens)
        .or(known.and_then(|m| m.context_tokens));

    let mut profile = json!({"provider": preset.provider, "model": model});
    if !base_url.is_empty() {
        profile["base_url"] = json!(base_url);
    }
    if let Some(key_name) = key_name {
        profile["key_name"] = json!(key_name);
    }
    if vision {
        profile["vision"] = json!(true);
    }
    if let Some(context) = context {
        profile["context_tokens"] = json!(context);
    }
    let _: Value = client
        .call(
            method::CONFIG_UPDATE,
            json!({"patch": {"profiles": {name.clone(): profile}}}),
        )
        .await?;
    writeln!(
        out,
        "Saved as `{name}`. Switch a chat to it with `/model {name}` (or add `--default`)."
    )?;
    Ok(name)
}

/// The model to use: from the provider's list, or typed when it has none.
fn choose(prompter: &mut impl Prompter, models: &[ModelInfo]) -> Result<String> {
    if models.is_empty() {
        return prompter.input("Model (the provider didn't list any)", None);
    }
    let mut shown: Vec<&ModelInfo> = models.iter().collect();
    if shown.len() > LONG_LIST {
        let filter = prompter
            .input(
                &format!(
                    "{} models; type part of a name (empty shows them all)",
                    shown.len()
                ),
                Some(""),
            )?
            .to_lowercase();
        shown.retain(|m| m.id.to_lowercase().contains(&filter));
        if shown.is_empty() {
            bail!("no model matches \"{filter}\"");
        }
    }
    let labels: Vec<String> = shown
        .iter()
        .map(|m| {
            let mut label = m.id.clone();
            if let Some(name) = &m.name {
                label += &format!(" ({name})");
            }
            if m.vision == Some(true) {
                label += " · sees pictures";
            }
            label
        })
        .collect();
    let labels: Vec<&str> = labels.iter().map(String::as_str).collect();
    Ok(shown[prompter.select("Which model?", &labels, 0)?]
        .id
        .clone())
}

/// A profile name from a model ID: "qwen2.5vl:7b" → "qwen2-5vl-7b".
fn profile_name(model: &str) -> String {
    let last = model.rsplit('/').next().unwrap_or(model);
    let mut name = String::new();
    for c in last.chars() {
        if c.is_ascii_alphanumeric() {
            name.push(c.to_ascii_lowercase());
        } else if !name.ends_with('-') {
            name.push('-');
        }
    }
    name.trim_matches('-').to_owned()
}
