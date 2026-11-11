//! Hosted voices, for a better-sounding Lucy than the local ones: OpenAI's and ElevenLabs'
//! text-to-speech, both asked for raw 24 kHz 16-bit PCM so it plays as it is.

use serde_json::json;

use crate::VoiceError;
use crate::engine::Audio;

const RATE: u32 = 24_000;

#[derive(Debug, Clone, PartialEq)]
pub enum Hosted {
    /// `voice`: one of OpenAI's voices, e.g. "nova".
    OpenAi { key: String, voice: String },
    /// `voice`: an ElevenLabs voice ID.
    ElevenLabs { key: String, voice: String },
}

impl Hosted {
    pub async fn speak(&self, client: &reqwest::Client, text: &str) -> Result<Audio, VoiceError> {
        let request = match self {
            Hosted::OpenAi { key, voice } => client
                .post("https://api.openai.com/v1/audio/speech")
                .bearer_auth(key)
                .json(&json!({
                    "model": "gpt-4o-mini-tts",
                    "voice": voice,
                    "input": text,
                    "response_format": "pcm",
                })),
            Hosted::ElevenLabs { key, voice } => client
                .post(format!(
                    "https://api.elevenlabs.io/v1/text-to-speech/{voice}?output_format=pcm_24000"
                ))
                .header("xi-api-key", key)
                .json(&json!({"text": text, "model_id": "eleven_flash_v2_5"})),
        };
        let response = request
            .send()
            .await
            .map_err(|err| VoiceError::Hosted(err.to_string()))?;
        let status = response.status();
        let bytes = response
            .bytes()
            .await
            .map_err(|err| VoiceError::Hosted(err.to_string()))?;
        if !status.is_success() {
            let said = String::from_utf8_lossy(&bytes);
            return Err(VoiceError::Hosted(format!("{status}: {}", said.trim())));
        }
        Ok(Audio {
            samples: pcm16(&bytes),
            sample_rate: RATE,
        })
    }
}

/// 16-bit little-endian PCM as samples.
fn pcm16(bytes: &[u8]) -> Vec<f32> {
    bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|b| f32::from(i16::from_le_bytes(*b)) / 32768.0)
        .collect()
}
