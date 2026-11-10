//! Lucy's voice, for her desktop app: hearing you (Whisper), knowing when you've stopped
//! talking (Silero), speaking (Kokoro, Piper, or a hosted voice) and her wake word, all on
//! sherpa-onnx. The library and models are downloaded the first time voice is turned on, into
//! Lucy's data folder, and the library is loaded at run time, so building Lucy needs none of it.

pub mod audio;
pub mod catalog;
pub mod engine;
pub mod fetch;
mod ffi;
pub mod hosted;

pub use catalog::Hearing;
pub use engine::{Audio, Ear, Engine, LocalVoice, Mouth, SAMPLE_RATE, Vad, WakeWord};
pub use fetch::{Pack, Progress};

#[derive(Debug, thiserror::Error)]
pub enum VoiceError {
    #[error("voice isn't available on this platform yet")]
    Unsupported,
    #[error("{0} isn't downloaded yet")]
    NotInstalled(String),
    #[error("couldn't download {0}: {1}")]
    Download(String, String),
    #[error("couldn't start voice: {0}")]
    Load(String),
    #[error("{0}")]
    Audio(String),
    #[error("the voice service said: {0}")]
    Hosted(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}
