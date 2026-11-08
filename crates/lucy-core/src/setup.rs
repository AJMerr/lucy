//! First-run setup: the model providers Lucy offers, shared by `lucy config init` and the app
//! (`setup.presets`).

use lucy_protocol::rpc::ProviderPreset;

use crate::config::ProviderKind;

struct Preset {
    label: &'static str,
    profile_name: &'static str,
    provider: ProviderKind,
    base_url: Option<&'static str>,
    model: Option<&'static str>,
    key_name: Option<&'static str>,
}

const PRESETS: &[Preset] = &[
    Preset {
        label: "Local Qwen via Ollama (free, private, runs on your GPU)",
        profile_name: "local-qwen",
        provider: ProviderKind::OpenAiCompat,
        base_url: Some("http://localhost:11434/v1"),
        model: Some("qwen3:14b"),
        key_name: None,
    },
    Preset {
        label: "Qwen cloud (Alibaba DashScope)",
        profile_name: "qwen-cloud",
        provider: ProviderKind::OpenAiCompat,
        base_url: Some("https://dashscope-intl.aliyuncs.com/compatible-mode/v1"),
        model: Some("qwen-max"),
        key_name: Some("dashscope"),
    },
    Preset {
        label: "OpenRouter (many models, one key)",
        profile_name: "openrouter",
        provider: ProviderKind::OpenAiCompat,
        base_url: Some("https://openrouter.ai/api/v1"),
        model: Some("qwen/qwen3-235b-a22b"),
        key_name: Some("openrouter"),
    },
    Preset {
        label: "Anthropic (Claude)",
        profile_name: "claude",
        provider: ProviderKind::Anthropic,
        base_url: None,
        model: Some("claude-sonnet-5-5"),
        key_name: Some("anthropic"),
    },
    // Any other OpenAI-compatible server: you give the URL, model and key name.
    Preset {
        label: "Other OpenAI-compatible endpoint",
        profile_name: "custom",
        provider: ProviderKind::OpenAiCompat,
        base_url: None,
        model: None,
        key_name: None,
    },
];

/// The providers to offer, in order; the last is a custom endpoint (no URL or model).
pub fn presets() -> Vec<ProviderPreset> {
    PRESETS
        .iter()
        .map(|preset| ProviderPreset {
            label: preset.label.into(),
            profile_name: preset.profile_name.into(),
            provider: match preset.provider {
                ProviderKind::OpenAiCompat => "openai_compat".into(),
                ProviderKind::Anthropic => "anthropic".into(),
            },
            base_url: preset.base_url.map(Into::into),
            model: preset.model.map(Into::into),
            key_name: preset.key_name.map(Into::into),
        })
        .collect()
}
