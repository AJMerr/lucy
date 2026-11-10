//! Builds a ready-to-use [`Provider`] from a config profile.

use lucy_providers::Provider;
use lucy_providers::anthropic::{Anthropic, AnthropicConfig};
use lucy_providers::generate::{ImageGenerator, OpenRouterImages};
use lucy_providers::openai_compat::{OpenAiCompat, OpenAiCompatConfig};

use crate::config::{Profile, ProviderKind};
use crate::secrets::{self, SecretStore, Secrets, SecretsError};

#[derive(Debug, thiserror::Error)]
pub enum BuildError {
    #[error(
        "profile `{profile}` needs API key `{key_name}`: run `lucy auth set {key_name}` \
         or set {env_var}"
    )]
    MissingKey {
        profile: String,
        key_name: String,
        env_var: String,
    },
    #[error("profile `{0}`: {1}")]
    Invalid(String, String),
    #[error(transparent)]
    Secrets(#[from] SecretsError),
}

/// Create the provider for profile `name`, resolving its API key if it names one.
pub fn build_provider<S, E>(
    name: &str,
    profile: &Profile,
    secrets: &Secrets<S, E>,
) -> Result<Box<dyn Provider>, BuildError>
where
    S: SecretStore,
    E: Fn(&str) -> Option<String>,
{
    let api_key = api_key(name, profile, secrets)?;

    Ok(match profile.provider {
        ProviderKind::OpenAiCompat => Box::new(OpenAiCompat::new(OpenAiCompatConfig {
            name: name.to_owned(),
            base_url: profile.base_url.clone().ok_or_else(|| {
                BuildError::Invalid(name.to_owned(), "openai_compat needs a base_url".into())
            })?,
            model: profile.model.clone(),
            api_key,
        })),
        ProviderKind::Anthropic => Box::new(Anthropic::new(AnthropicConfig {
            name: name.to_owned(),
            base_url: profile.base_url.clone(),
            model: profile.model.clone(),
            api_key: api_key.ok_or_else(|| {
                BuildError::Invalid(name.to_owned(), "anthropic needs a key_name".into())
            })?,
        })),
    })
}

/// Create an image generator from profile `name` (an OpenAI-compatible image model, like
/// OpenRouter's).
pub fn build_generator<S, E>(
    name: &str,
    profile: &Profile,
    secrets: &Secrets<S, E>,
) -> Result<Box<dyn ImageGenerator>, BuildError>
where
    S: SecretStore,
    E: Fn(&str) -> Option<String>,
{
    if profile.provider != ProviderKind::OpenAiCompat {
        return Err(BuildError::Invalid(
            name.to_owned(),
            "only openai_compat profiles (like OpenRouter's image models) can make pictures".into(),
        ));
    }
    Ok(Box::new(OpenRouterImages::new(OpenAiCompatConfig {
        name: name.to_owned(),
        base_url: profile.base_url.clone().ok_or_else(|| {
            BuildError::Invalid(name.to_owned(), "openai_compat needs a base_url".into())
        })?,
        model: profile.model.clone(),
        api_key: api_key(name, profile, secrets)?,
    })))
}

fn api_key<S, E>(
    name: &str,
    profile: &Profile,
    secrets: &Secrets<S, E>,
) -> Result<Option<String>, BuildError>
where
    S: SecretStore,
    E: Fn(&str) -> Option<String>,
{
    let Some(key_name) = &profile.key_name else {
        return Ok(None);
    };
    let secret = secrets
        .resolve(key_name)?
        .ok_or_else(|| BuildError::MissingKey {
            profile: name.to_owned(),
            key_name: key_name.clone(),
            env_var: secrets::env_var_name(key_name),
        })?;
    Ok(Some(secret.value))
}
