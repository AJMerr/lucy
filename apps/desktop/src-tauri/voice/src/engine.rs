//! sherpa-onnx, loaded from the voice folder at run time, and safe wrappers for what Lucy uses:
//! Whisper (hearing), Silero (when you've stopped talking), Kokoro and Piper (speaking), and a
//! keyword spotter (her wake word).

use std::ffi::{CStr, CString, c_char};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::VoiceError;
use crate::catalog::{self, Hearing};
use crate::ffi::{self, SherpaOnnx};

/// Audio Lucy hears at: 16 kHz mono.
pub const SAMPLE_RATE: u32 = 16_000;

const THREADS: i32 = 2;

/// The loaded library.
#[derive(Clone)]
pub struct Engine {
    lib: Arc<SherpaOnnx>,
    base: PathBuf,
}

impl Engine {
    /// Load sherpa-onnx from the voice folder `base` (installed by `catalog::runtime()`).
    pub fn load(base: &Path) -> Result<Self, VoiceError> {
        let (pack, lib_path) = catalog::runtime().ok_or(VoiceError::Unsupported)?;
        let dir = base.join(pack.installs);
        let lib_file = dir.join(lib_path);
        if !lib_file.exists() {
            return Err(VoiceError::NotInstalled(pack.label.into()));
        }
        // ONNX Runtime first, so the C API finds it already loaded (macOS, Windows).
        let lib_dir = lib_file.parent().unwrap_or(&dir);
        let mut keep = Vec::new();
        for name in [
            "libonnxruntime.dylib",
            "onnxruntime.dll",
            "onnxruntime_providers_shared.dll",
        ] {
            let path = lib_dir.join(name);
            if path.exists() {
                // SAFETY: a library from sherpa-onnx's release, loaded for its symbols.
                if let Ok(library) = unsafe { libloading::Library::new(&path) } {
                    keep.push(library);
                }
            }
        }
        // SAFETY: the C API library the bindings in `ffi.rs` were generated from.
        let lib = unsafe { SherpaOnnx::new(&lib_file) }
            .map_err(|err| VoiceError::Load(err.to_string()))?;
        // The preloaded libraries stay loaded for the process (the C API refers to them).
        std::mem::forget(keep);
        Ok(Self {
            lib: Arc::new(lib),
            base: base.to_owned(),
        })
    }

    fn path(&self, parts: &[&str]) -> Result<CString, VoiceError> {
        let mut path = self.base.clone();
        for part in parts {
            path.push(part);
        }
        if !path.exists() {
            return Err(VoiceError::NotInstalled(path.display().to_string()));
        }
        Ok(cstring(&path.to_string_lossy()))
    }
}

fn cstring(text: &str) -> CString {
    CString::new(text.replace('\0', " ")).expect("no NULs left")
}

// --- Hearing ----------------------------------------------------------------------------------

/// Whisper, turning what you said into text.
pub struct Ear {
    lib: Arc<SherpaOnnx>,
    recognizer: *const ffi::SherpaOnnxOfflineRecognizer,
}

// SAFETY: sherpa-onnx's recognizer may be used from any thread, one call at a time (`&mut`).
unsafe impl Send for Ear {}

impl Ear {
    pub fn new(engine: &Engine, hearing: Hearing) -> Result<Self, VoiceError> {
        let dir = hearing.pack().installs;
        let name = hearing.name();
        let encoder = engine.path(&[dir, &format!("{name}-encoder.int8.onnx")])?;
        let decoder = engine.path(&[dir, &format!("{name}-decoder.int8.onnx")])?;
        let tokens = engine.path(&[dir, &format!("{name}-tokens.txt")])?;
        let task = cstring("transcribe");
        let empty = cstring("");
        let greedy = cstring("greedy_search");
        let cpu = cstring("cpu");
        // SAFETY: a plain C struct; zero is "unset" for every field sherpa-onnx reads.
        let mut config: ffi::SherpaOnnxOfflineRecognizerConfig = unsafe { std::mem::zeroed() };
        config.feat_config.sample_rate = SAMPLE_RATE as i32;
        config.feat_config.feature_dim = 80;
        config.model_config.whisper.encoder = encoder.as_ptr();
        config.model_config.whisper.decoder = decoder.as_ptr();
        // Empty: Whisper works out the language itself.
        config.model_config.whisper.language = empty.as_ptr();
        config.model_config.whisper.task = task.as_ptr();
        config.model_config.whisper.tail_paddings = -1;
        config.model_config.tokens = tokens.as_ptr();
        config.model_config.num_threads = THREADS;
        config.model_config.provider = cpu.as_ptr();
        config.decoding_method = greedy.as_ptr();
        // SAFETY: the config and its strings outlive the call; sherpa-onnx copies them.
        let recognizer = unsafe { engine.lib.SherpaOnnxCreateOfflineRecognizer(&config) };
        if recognizer.is_null() {
            return Err(VoiceError::Load(format!("Whisper {name} wouldn't load")));
        }
        Ok(Self {
            lib: Arc::clone(&engine.lib),
            recognizer,
        })
    }

    /// What was said in `samples` (16 kHz mono).
    pub fn transcribe(&mut self, samples: &[f32]) -> String {
        if samples.is_empty() {
            return String::new();
        }
        // SAFETY: the recognizer is alive; the stream and result are freed before returning.
        unsafe {
            let stream = self.lib.SherpaOnnxCreateOfflineStream(self.recognizer);
            self.lib.SherpaOnnxAcceptWaveformOffline(
                stream,
                SAMPLE_RATE as i32,
                samples.as_ptr(),
                samples.len() as i32,
            );
            self.lib
                .SherpaOnnxDecodeOfflineStream(self.recognizer, stream);
            let result = self.lib.SherpaOnnxGetOfflineStreamResult(stream);
            let text = if result.is_null() {
                String::new()
            } else {
                text_of((*result).text)
            };
            self.lib.SherpaOnnxDestroyOfflineRecognizerResult(result);
            self.lib.SherpaOnnxDestroyOfflineStream(stream);
            text.trim().to_owned()
        }
    }
}

impl Drop for Ear {
    fn drop(&mut self) {
        // SAFETY: created by this wrapper, freed once.
        unsafe { self.lib.SherpaOnnxDestroyOfflineRecognizer(self.recognizer) };
    }
}

/// # Safety
/// `text` is null or a NUL-terminated string.
unsafe fn text_of(text: *const c_char) -> String {
    if text.is_null() {
        return String::new();
    }
    // SAFETY: the caller's promise.
    unsafe { CStr::from_ptr(text) }
        .to_string_lossy()
        .into_owned()
}

// --- When you've stopped talking --------------------------------------------------------------

/// Silero: notices speech, and when it's over.
pub struct Vad {
    lib: Arc<SherpaOnnx>,
    vad: *const ffi::SherpaOnnxVoiceActivityDetector,
    heard_speech: bool,
}

// SAFETY: used from one thread at a time (`&mut`).
unsafe impl Send for Vad {}

impl Vad {
    /// `silence`: how long a pause ends what you're saying, in seconds.
    pub fn new(engine: &Engine, silence: f32) -> Result<Self, VoiceError> {
        let model = engine.path(&[catalog::vad().installs])?;
        let cpu = cstring("cpu");
        // SAFETY: plain C struct, zero is unset.
        let mut config: ffi::SherpaOnnxVadModelConfig = unsafe { std::mem::zeroed() };
        config.silero_vad.model = model.as_ptr();
        config.silero_vad.threshold = 0.5;
        config.silero_vad.min_silence_duration = silence;
        config.silero_vad.min_speech_duration = 0.25;
        config.silero_vad.window_size = 512;
        config.silero_vad.max_speech_duration = 30.0;
        config.sample_rate = SAMPLE_RATE as i32;
        config.num_threads = 1;
        config.provider = cpu.as_ptr();
        // SAFETY: as above.
        let vad = unsafe {
            engine
                .lib
                .SherpaOnnxCreateVoiceActivityDetector(&config, 60.0)
        };
        if vad.is_null() {
            return Err(VoiceError::Load("the speech detector wouldn't load".into()));
        }
        Ok(Self {
            lib: Arc::clone(&engine.lib),
            vad,
            heard_speech: false,
        })
    }

    /// Feed audio; true once some speech has been heard and has ended.
    pub fn finished(&mut self, samples: &[f32]) -> bool {
        // SAFETY: the detector is alive; segments are freed as they're taken.
        unsafe {
            self.lib.SherpaOnnxVoiceActivityDetectorAcceptWaveform(
                self.vad,
                samples.as_ptr(),
                samples.len() as i32,
            );
            if self.lib.SherpaOnnxVoiceActivityDetectorDetected(self.vad) != 0 {
                self.heard_speech = true;
            }
            let mut ended = false;
            while self.lib.SherpaOnnxVoiceActivityDetectorEmpty(self.vad) == 0 {
                let segment = self.lib.SherpaOnnxVoiceActivityDetectorFront(self.vad);
                self.lib.SherpaOnnxDestroySpeechSegment(segment);
                self.lib.SherpaOnnxVoiceActivityDetectorPop(self.vad);
                ended = true;
            }
            ended && self.heard_speech
        }
    }

    pub fn reset(&mut self) {
        self.heard_speech = false;
        // SAFETY: the detector is alive.
        unsafe { self.lib.SherpaOnnxVoiceActivityDetectorReset(self.vad) };
    }
}

impl Drop for Vad {
    fn drop(&mut self) {
        // SAFETY: created by this wrapper, freed once.
        unsafe { self.lib.SherpaOnnxDestroyVoiceActivityDetector(self.vad) };
    }
}

// --- Speaking ---------------------------------------------------------------------------------

/// A local voice: Kokoro (one of its speakers) or a Piper voice.
#[derive(Debug, Clone, PartialEq)]
pub enum LocalVoice {
    Kokoro { speaker: String },
    Piper { voice: String },
}

/// Speech: samples at a sample rate.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Audio {
    pub samples: Vec<f32>,
    pub sample_rate: u32,
}

pub struct Mouth {
    lib: Arc<SherpaOnnx>,
    tts: *const ffi::SherpaOnnxOfflineTts,
    speaker: i32,
}

// SAFETY: used from one thread at a time (`&mut`).
unsafe impl Send for Mouth {}

impl Mouth {
    pub fn new(engine: &Engine, voice: &LocalVoice) -> Result<Self, VoiceError> {
        let cpu = cstring("cpu");
        // SAFETY: plain C struct, zero is unset.
        let mut config: ffi::SherpaOnnxOfflineTtsConfig = unsafe { std::mem::zeroed() };
        config.model.num_threads = THREADS;
        config.model.provider = cpu.as_ptr();
        config.max_num_sentences = 1;
        let mut strings = Vec::new();
        let speaker = match voice {
            LocalVoice::Kokoro { speaker } => {
                let dir = catalog::kokoro().installs;
                let model = engine.path(&[dir, "model.onnx"])?;
                let voices = engine.path(&[dir, "voices.bin"])?;
                let tokens = engine.path(&[dir, "tokens.txt"])?;
                let data = engine.path(&[dir, "espeak-ng-data"])?;
                config.model.kokoro.model = model.as_ptr();
                config.model.kokoro.voices = voices.as_ptr();
                config.model.kokoro.tokens = tokens.as_ptr();
                config.model.kokoro.data_dir = data.as_ptr();
                config.model.kokoro.length_scale = 1.0;
                strings.extend([model, voices, tokens, data]);
                catalog::KOKORO_SPEAKERS
                    .iter()
                    .position(|s| s == speaker)
                    .unwrap_or(3) as i32
            }
            LocalVoice::Piper { voice } => {
                let pack = catalog::piper(voice)
                    .ok_or_else(|| VoiceError::NotInstalled(format!("Piper voice {voice}")))?;
                let dir = pack.installs;
                let model = engine.path(&[dir, &format!("{voice}.onnx")])?;
                let tokens = engine.path(&[dir, "tokens.txt"])?;
                let data = engine.path(&[dir, "espeak-ng-data"])?;
                config.model.vits.model = model.as_ptr();
                config.model.vits.tokens = tokens.as_ptr();
                config.model.vits.data_dir = data.as_ptr();
                config.model.vits.noise_scale = 0.667;
                config.model.vits.noise_scale_w = 0.8;
                config.model.vits.length_scale = 1.0;
                strings.extend([model, tokens, data]);
                0
            }
        };
        // SAFETY: the config and `strings` outlive the call; sherpa-onnx copies them.
        let tts = unsafe { engine.lib.SherpaOnnxCreateOfflineTts(&config) };
        drop(strings);
        if tts.is_null() {
            return Err(VoiceError::Load("the voice wouldn't load".into()));
        }
        Ok(Self {
            lib: Arc::clone(&engine.lib),
            tts,
            speaker,
        })
    }

    pub fn speak(&mut self, text: &str) -> Audio {
        let text = cstring(text);
        // SAFETY: the voice is alive; the generated audio is copied, then freed.
        unsafe {
            let audio =
                self.lib
                    .SherpaOnnxOfflineTtsGenerate(self.tts, text.as_ptr(), self.speaker, 1.0);
            if audio.is_null() {
                return Audio::default();
            }
            let generated = &*audio;
            let samples = if generated.samples.is_null() || generated.n <= 0 {
                Vec::new()
            } else {
                std::slice::from_raw_parts(generated.samples, generated.n as usize).to_vec()
            };
            let sample_rate = generated.sample_rate.max(1) as u32;
            self.lib.SherpaOnnxDestroyOfflineTtsGeneratedAudio(audio);
            Audio {
                samples,
                sample_rate,
            }
        }
    }
}

impl Drop for Mouth {
    fn drop(&mut self) {
        // SAFETY: created by this wrapper, freed once.
        unsafe { self.lib.SherpaOnnxDestroyOfflineTts(self.tts) };
    }
}

// --- Her wake word ------------------------------------------------------------------------------

/// Listens for "Hey Lucy".
pub struct WakeWord {
    lib: Arc<SherpaOnnx>,
    spotter: *const ffi::SherpaOnnxKeywordSpotter,
    stream: *const ffi::SherpaOnnxOnlineStream,
}

// SAFETY: used from one thread at a time (`&mut`).
unsafe impl Send for WakeWord {}

impl WakeWord {
    pub fn new(engine: &Engine) -> Result<Self, VoiceError> {
        let dir = catalog::wake_word().installs;
        let encoder = engine.path(&[dir, "encoder-epoch-12-avg-2-chunk-16-left-64.int8.onnx"])?;
        let decoder = engine.path(&[dir, "decoder-epoch-12-avg-2-chunk-16-left-64.int8.onnx"])?;
        let joiner = engine.path(&[dir, "joiner-epoch-12-avg-2-chunk-16-left-64.int8.onnx"])?;
        let tokens = engine.path(&[dir, "tokens.txt"])?;
        let cpu = cstring("cpu");
        let keywords = cstring(catalog::WAKE_PHRASE);
        // SAFETY: plain C struct, zero is unset.
        let mut config: ffi::SherpaOnnxKeywordSpotterConfig = unsafe { std::mem::zeroed() };
        config.feat_config.sample_rate = SAMPLE_RATE as i32;
        config.feat_config.feature_dim = 80;
        config.model_config.transducer.encoder = encoder.as_ptr();
        config.model_config.transducer.decoder = decoder.as_ptr();
        config.model_config.transducer.joiner = joiner.as_ptr();
        config.model_config.tokens = tokens.as_ptr();
        config.model_config.num_threads = 1;
        config.model_config.provider = cpu.as_ptr();
        config.max_active_paths = 4;
        config.num_trailing_blanks = 1;
        config.keywords_score = 1.0;
        config.keywords_threshold = 0.25;
        config.keywords_buf = keywords.as_ptr();
        config.keywords_buf_size = keywords.as_bytes().len() as i32;
        // SAFETY: the config and its strings outlive the call.
        let spotter = unsafe { engine.lib.SherpaOnnxCreateKeywordSpotter(&config) };
        if spotter.is_null() {
            return Err(VoiceError::Load("the wake word wouldn't load".into()));
        }
        // SAFETY: the spotter was just created.
        let stream = unsafe { engine.lib.SherpaOnnxCreateKeywordStream(spotter) };
        Ok(Self {
            lib: Arc::clone(&engine.lib),
            spotter,
            stream,
        })
    }

    /// Feed audio (16 kHz mono); true when "Hey Lucy" was just said.
    pub fn heard(&mut self, samples: &[f32]) -> bool {
        // SAFETY: the spotter and stream are alive; results are freed after reading.
        unsafe {
            self.lib.SherpaOnnxOnlineStreamAcceptWaveform(
                self.stream,
                SAMPLE_RATE as i32,
                samples.as_ptr(),
                samples.len() as i32,
            );
            let mut heard = false;
            while self
                .lib
                .SherpaOnnxIsKeywordStreamReady(self.spotter, self.stream)
                != 0
            {
                self.lib
                    .SherpaOnnxDecodeKeywordStream(self.spotter, self.stream);
                let result = self
                    .lib
                    .SherpaOnnxGetKeywordResult(self.spotter, self.stream);
                if !result.is_null() {
                    if !text_of((*result).keyword).is_empty() {
                        heard = true;
                        // Start afresh, so one "Hey Lucy" counts once.
                        self.lib
                            .SherpaOnnxResetKeywordStream(self.spotter, self.stream);
                    }
                    self.lib.SherpaOnnxDestroyKeywordResult(result);
                }
            }
            heard
        }
    }
}

impl Drop for WakeWord {
    fn drop(&mut self) {
        // SAFETY: created by this wrapper, freed once, stream first.
        unsafe {
            self.lib.SherpaOnnxDestroyOnlineStream(self.stream);
            self.lib.SherpaOnnxDestroyKeywordSpotter(self.spotter);
        }
    }
}
