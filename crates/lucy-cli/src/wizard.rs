//! First-run setup: asks which provider and model to use and builds a [`Config`].

use anyhow::Result;
use lucy_core::config::{Config, Profile, ProviderKind};

/// The questions the wizard can ask. Real terminal in production, scripted in tests.
pub trait Prompter {
    fn select(&mut self, prompt: &str, items: &[&str], default: usize) -> Result<usize>;
    /// Free text. With a default, empty input means the default.
    fn input(&mut self, prompt: &str, default: Option<&str>) -> Result<String>;
}

struct Preset {
    label: &'static str,
    profile_name: &'static str,
    provider: ProviderKind,
    /// `None` for a custom OpenAI-compatible endpoint: ask for URL, model and key name.
    endpoint: Option<Endpoint>,
}

struct Endpoint {
    base_url: Option<&'static str>,
    model: &'static str,
    key_name: Option<&'static str>,
}

const PRESETS: &[Preset] = &[
    Preset {
        label: "Local Qwen via Ollama (free, private, runs on your GPU)",
        profile_name: "local-qwen",
        provider: ProviderKind::OpenAiCompat,
        endpoint: Some(Endpoint {
            base_url: Some("http://localhost:11434/v1"),
            model: "qwen3:14b",
            key_name: None,
        }),
    },
    Preset {
        label: "Qwen cloud (Alibaba DashScope)",
        profile_name: "qwen-cloud",
        provider: ProviderKind::OpenAiCompat,
        endpoint: Some(Endpoint {
            base_url: Some("https://dashscope-intl.aliyuncs.com/compatible-mode/v1"),
            model: "qwen-max",
            key_name: Some("dashscope"),
        }),
    },
    Preset {
        label: "OpenRouter (many models, one key)",
        profile_name: "openrouter",
        provider: ProviderKind::OpenAiCompat,
        endpoint: Some(Endpoint {
            base_url: Some("https://openrouter.ai/api/v1"),
            model: "qwen/qwen3-235b-a22b",
            key_name: Some("openrouter"),
        }),
    },
    Preset {
        label: "Anthropic (Claude)",
        profile_name: "claude",
        provider: ProviderKind::Anthropic,
        endpoint: Some(Endpoint {
            base_url: None,
            model: "claude-sonnet-5-5",
            key_name: Some("anthropic"),
        }),
    },
    Preset {
        label: "Other OpenAI-compatible endpoint",
        profile_name: "custom",
        provider: ProviderKind::OpenAiCompat,
        endpoint: None,
    },
];

/// Ask the first-run questions and build a config with a single default profile.
pub fn run(prompter: &mut impl Prompter) -> Result<Config> {
    let labels: Vec<_> = PRESETS.iter().map(|preset| preset.label).collect();
    let preset = &PRESETS[prompter.select("Which model provider should Lucy use?", &labels, 0)?];

    let profile = match &preset.endpoint {
        Some(endpoint) => Profile {
            provider: preset.provider,
            base_url: endpoint.base_url.map(Into::into),
            model: prompter.input("Model", Some(endpoint.model))?,
            key_name: endpoint.key_name.map(Into::into),
        },
        None => {
            let base_url = prompter.input("Base URL (e.g. http://localhost:8080/v1)", None)?;
            let model = prompter.input("Model", None)?;
            let key_name = prompter.input("API key name (leave empty if none)", Some(""))?;
            Profile {
                provider: preset.provider,
                base_url: Some(base_url),
                model,
                key_name: Some(key_name).filter(|name| !name.is_empty()),
            }
        }
    };

    let name = prompter.input("Profile name", Some(preset.profile_name))?;
    let mut config = Config::with_profile(name, profile);

    let user_name = prompter.input("What should Lucy call you? (leave empty to skip)", Some(""))?;
    config.user.name = Some(user_name.trim().to_owned()).filter(|n| !n.is_empty());
    Ok(config)
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;

    use lucy_core::config::{Profile, ProviderKind};

    use super::*;

    enum Answer {
        Pick(&'static str),
        Type(&'static str),
        Enter,
    }

    struct Scripted(VecDeque<Answer>);

    impl Scripted {
        fn new(answers: impl IntoIterator<Item = Answer>) -> Self {
            Self(answers.into_iter().collect())
        }
    }

    impl Prompter for Scripted {
        fn select(&mut self, prompt: &str, items: &[&str], default: usize) -> Result<usize> {
            match self.0.pop_front() {
                Some(Answer::Pick(label)) => Ok(items
                    .iter()
                    .position(|item| item.contains(label))
                    .unwrap_or_else(|| panic!("{prompt}: no item matching {label:?}"))),
                Some(Answer::Enter) => Ok(default),
                _ => panic!("{prompt}: expected a select answer"),
            }
        }

        fn input(&mut self, prompt: &str, default: Option<&str>) -> Result<String> {
            match self.0.pop_front() {
                Some(Answer::Type(text)) => Ok(text.into()),
                Some(Answer::Enter) => Ok(default.unwrap_or_default().into()),
                _ => panic!("{prompt}: expected an input answer"),
            }
        }
    }

    fn run_scripted(answers: impl IntoIterator<Item = Answer>) -> Config {
        let mut prompter = Scripted::new(answers);
        let config = run(&mut prompter).unwrap();
        assert!(
            prompter.0.is_empty(),
            "wizard asked fewer questions than scripted"
        );
        config
    }

    #[test]
    fn accepting_every_default_sets_up_local_qwen() {
        let config = run_scripted([Answer::Enter, Answer::Enter, Answer::Enter, Answer::Enter]);

        assert_eq!(
            config,
            Config::with_profile(
                "local-qwen",
                Profile {
                    provider: ProviderKind::OpenAiCompat,
                    base_url: Some("http://localhost:11434/v1".into()),
                    model: "qwen3:14b".into(),
                    key_name: None,
                },
            )
        );
    }

    #[test]
    fn presets_fill_in_provider_url_and_key_name() {
        let cases = [
            (
                "Qwen cloud",
                "qwen-cloud",
                ProviderKind::OpenAiCompat,
                Some("https://dashscope-intl.aliyuncs.com/compatible-mode/v1"),
                "qwen-max",
                Some("dashscope"),
            ),
            (
                "OpenRouter",
                "openrouter",
                ProviderKind::OpenAiCompat,
                Some("https://openrouter.ai/api/v1"),
                "qwen/qwen3-235b-a22b",
                Some("openrouter"),
            ),
            (
                "Anthropic",
                "claude",
                ProviderKind::Anthropic,
                None,
                "claude-sonnet-5-5",
                Some("anthropic"),
            ),
        ];

        for (preset, name, provider, base_url, model, key_name) in cases {
            let config = run_scripted([
                Answer::Pick(preset),
                Answer::Enter,
                Answer::Enter,
                Answer::Enter,
            ]);
            assert_eq!(
                config,
                Config::with_profile(
                    name,
                    Profile {
                        provider,
                        base_url: base_url.map(Into::into),
                        model: model.into(),
                        key_name: key_name.map(Into::into),
                    },
                ),
                "preset {preset}"
            );
        }
    }

    #[test]
    fn typed_model_and_profile_name_override_preset_defaults() {
        let config = run_scripted([
            Answer::Pick("Anthropic"),
            Answer::Type("claude-opus-5-5"),
            Answer::Type("opus"),
            Answer::Enter,
        ]);

        assert_eq!(config.defaults.profile, "opus");
        assert_eq!(config.profiles["opus"].model, "claude-opus-5-5");
    }

    #[test]
    fn custom_endpoint_asks_for_url_and_optional_key_name() {
        let config = run_scripted([
            Answer::Pick("Other OpenAI-compatible"),
            Answer::Type("http://192.168.1.20:8080/v1"),
            Answer::Type("llama-3.3-70b"),
            Answer::Enter,
            Answer::Type("homelab"),
            Answer::Enter,
        ]);

        assert_eq!(
            config,
            Config::with_profile(
                "homelab",
                Profile {
                    provider: ProviderKind::OpenAiCompat,
                    base_url: Some("http://192.168.1.20:8080/v1".into()),
                    model: "llama-3.3-70b".into(),
                    key_name: None,
                },
            )
        );
    }

    #[test]
    fn name_is_asked_last_and_optional() {
        let named = run_scripted([
            Answer::Enter,
            Answer::Enter,
            Answer::Enter,
            Answer::Type("Sam"),
        ]);
        assert_eq!(named.user.name.as_deref(), Some("Sam"));

        let unnamed = run_scripted([Answer::Enter, Answer::Enter, Answer::Enter, Answer::Enter]);
        assert_eq!(unnamed.user.name, None);
    }
}
