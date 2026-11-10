//! Everything voice downloads: sherpa-onnx's library for this platform, and the models.

use crate::fetch::Pack;

/// The sherpa-onnx release the bindings (`ffi.rs`) were generated from.
pub const VERSION: &str = "1.12.9";

const RELEASES: &str = "https://github.com/k2-fsa/sherpa-onnx/releases/download";

/// sherpa-onnx's shared library (with ONNX Runtime) for this platform, and where in it the
/// C API's library is. `None` where no build is published.
pub fn runtime() -> Option<(Pack, &'static str)> {
    let (archive, sha256, size, lib) = if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        (
            "sherpa-onnx-v1.12.9-linux-x64-shared.tar.bz2",
            "cd22ee337e205674536643d2bc86e984fbbbf9c5c52743c4df6fc8d8dd17371b",
            24_606_687,
            "lib/libsherpa-onnx-c-api.so",
        )
    } else if cfg!(all(target_os = "linux", target_arch = "aarch64")) {
        (
            "sherpa-onnx-v1.12.9-linux-aarch64-shared-cpu.tar.bz2",
            "ebb5686bbdc3e35dc9d84ead46dd52fe5e9174c0cd51e98ec649b82165a1fa88",
            20_000_000,
            "lib/libsherpa-onnx-c-api.so",
        )
    } else if cfg!(target_os = "macos") {
        (
            "sherpa-onnx-v1.12.9-osx-universal2-shared.tar.bz2",
            "e3f3596b84c65eef8939a8a45ced3d7c4ed857455ba5ae1148ce7a1c93ff9a3c",
            40_000_000,
            "lib/libsherpa-onnx-c-api.dylib",
        )
    } else if cfg!(all(target_os = "windows", target_arch = "x86_64")) {
        (
            "sherpa-onnx-v1.12.9-win-x64-shared.tar.bz2",
            "57a0dff63c468ed067c6ffd1a9c9b6f66f4cf73304d44b5f17518010ce2e0977",
            30_000_000,
            "lib/sherpa-onnx-c-api.dll",
        )
    } else {
        return None;
    };
    let url = format!("{RELEASES}/v{VERSION}/{archive}");
    Some((
        Pack {
            label: "Lucy's voice engine",
            url,
            installs: "runtime-1.12.9",
            size,
            sha256: Some(sha256),
        },
        lib,
    ))
}

/// Whisper sizes for hearing you: bigger is more accurate and slower.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Hearing {
    Tiny,
    Base,
    Small,
    Turbo,
}

impl Hearing {
    pub fn name(self) -> &'static str {
        match self {
            Hearing::Tiny => "tiny",
            Hearing::Base => "base",
            Hearing::Small => "small",
            Hearing::Turbo => "turbo",
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        [Hearing::Tiny, Hearing::Base, Hearing::Small, Hearing::Turbo]
            .into_iter()
            .find(|h| h.name() == name)
    }

    pub fn pack(self) -> Pack {
        let (url, installs, size, label) = match self {
            Hearing::Tiny => (
                "https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/sherpa-onnx-whisper-tiny.tar.bz2",
                "sherpa-onnx-whisper-tiny",
                116_204_861,
                "Whisper tiny",
            ),
            Hearing::Base => (
                "https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/sherpa-onnx-whisper-base.tar.bz2",
                "sherpa-onnx-whisper-base",
                207_557_382,
                "Whisper base",
            ),
            Hearing::Small => (
                "https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/sherpa-onnx-whisper-small.tar.bz2",
                "sherpa-onnx-whisper-small",
                639_387_718,
                "Whisper small",
            ),
            Hearing::Turbo => (
                "https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/sherpa-onnx-whisper-turbo.tar.bz2",
                "sherpa-onnx-whisper-turbo",
                563_790_207,
                "Whisper turbo",
            ),
        };
        Pack {
            label,
            url: url.into(),
            installs,
            size,
            sha256: None,
        }
    }
}

/// Silero, which tells when you've stopped talking.
pub fn vad() -> Pack {
    Pack {
        label: "Speech detector",
        url: format!("{RELEASES}/asr-models/silero_vad.onnx"),
        installs: "silero_vad.onnx",
        size: 643_854,
        sha256: None,
    }
}

/// The keyword spotter that hears "Hey Lucy".
pub fn wake_word() -> Pack {
    Pack {
        label: "Wake word",
        url: format!(
            "{RELEASES}/kws-models/sherpa-onnx-kws-zipformer-gigaspeech-3.3M-2024-01-01.tar.bz2"
        ),
        installs: "sherpa-onnx-kws-zipformer-gigaspeech-3.3M-2024-01-01",
        size: 17_626_723,
        sha256: None,
    }
}

/// "Hey Lucy" in the wake-word model's word pieces (its `bpe.model`), with its name.
pub const WAKE_PHRASE: &str = "▁HE Y ▁ LU C Y @HEY_LUCY";

/// Local voices.
pub fn kokoro() -> Pack {
    Pack {
        label: "Kokoro voices",
        url: format!("{RELEASES}/tts-models/kokoro-en-v0_19.tar.bz2"),
        installs: "kokoro-en-v0_19",
        size: 319_625_534,
        sha256: None,
    }
}

/// Kokoro's speakers (`kokoro-en-v0_19`), in speaker-ID order.
pub const KOKORO_SPEAKERS: &[&str] = &[
    "af",
    "af_bella",
    "af_nicole",
    "af_sarah",
    "af_sky",
    "am_adam",
    "am_michael",
    "bf_emma",
    "bf_isabella",
    "bm_george",
    "bm_lewis",
];

/// Piper voices offered (each its own download).
pub const PIPER_VOICES: &[&str] = &["en_US-amy-medium", "en_GB-jenny_dioco-medium"];

pub fn piper(voice: &str) -> Option<Pack> {
    let (url, installs) = match voice {
        "en_US-amy-medium" => (
            "https://github.com/k2-fsa/sherpa-onnx/releases/download/tts-models/vits-piper-en_US-amy-medium.tar.bz2",
            "vits-piper-en_US-amy-medium",
        ),
        "en_GB-jenny_dioco-medium" => (
            "https://github.com/k2-fsa/sherpa-onnx/releases/download/tts-models/vits-piper-en_GB-jenny_dioco-medium.tar.bz2",
            "vits-piper-en_GB-jenny_dioco-medium",
        ),
        _ => return None,
    };
    Some(Pack {
        label: "Piper voice",
        url: url.into(),
        installs,
        size: 67_225_000,
        sha256: None,
    })
}
