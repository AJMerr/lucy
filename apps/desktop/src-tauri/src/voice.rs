//! Lucy's voice in her app (`[voice]` in config.toml): push-to-talk and the wake word on one
//! thread (the microphone, Whisper, Silero, the keyword spotter), and her voice on another (a
//! local one or a hosted one, and the speakers). The pages drive it with commands and hear back
//! through events:
//!
//! - `voice-progress` `{label, done, total}` while downloading the first time
//! - `voice-state` `"listening" | "thinking" | "idle"`
//! - `voice-heard` `{origin, text}` when you've finished talking
//! - `voice-wake` when "Hey Lucy" was heard (the sprite starts listening)
//! - `voice-error` `{message}`
//! - `voice-status` (a `Status`) when voice is (re)started or turned off

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use lucy_core::config::{Config, VoiceSettings};
use lucy_core::secrets::SystemSecrets;
use lucy_voice::audio::{Mic, Speaker, SpeakerHandle};
use lucy_voice::catalog::{self, Hearing};
use lucy_voice::hosted::Hosted;
use lucy_voice::{Ear, Engine, LocalVoice, Mouth, Pack, Vad, WakeWord};
use serde_json::json;
use tauri::{AppHandle, Emitter, Manager};

/// How long a pause ends what you're saying (when not holding the key).
const SILENCE_SECS: f32 = 0.8;

/// The voice service, managed by Tauri.
pub struct Voice {
    ear: Mutex<Option<mpsc::Sender<EarCommand>>>,
    mouth: Arc<MouthQueue>,
    state: Mutex<Status>,
}

#[derive(Clone, Default, serde::Serialize)]
pub struct Status {
    /// `[voice] enabled`.
    pub enabled: bool,
    /// Downloaded and loaded.
    pub ready: bool,
    /// Why it isn't ready, if it failed.
    pub error: Option<String>,
    pub read_typed: bool,
    pub wake_word: bool,
}

enum EarCommand {
    Listen { hold: bool, origin: String },
    Stop,
    Wake(bool),
}

/// What's waiting to be said; `generation` changes on `hush`, so queued speech is dropped.
#[derive(Default)]
struct MouthQueue {
    pending: Mutex<(u64, VecDeque<(u64, String)>)>,
    ready: Condvar,
    speaker: Mutex<Option<SpeakerHandle>>,
}

impl Voice {
    pub fn new() -> Self {
        Self {
            ear: Mutex::new(None),
            mouth: Arc::default(),
            state: Mutex::default(),
        }
    }
}

fn voice_dir() -> Option<PathBuf> {
    lucy_core::sessions::default_path().and_then(|db| db.parent().map(|dir| dir.join("voice")))
}

fn settings() -> VoiceSettings {
    lucy_core::config::default_path()
        .and_then(|path| Config::load(&path).ok().flatten())
        .map(|config| config.voice)
        .unwrap_or_default()
}

fn local_voice(settings: &VoiceSettings) -> Option<LocalVoice> {
    match settings.voice.as_str() {
        "kokoro" => Some(LocalVoice::Kokoro {
            speaker: settings
                .speaker
                .clone()
                .unwrap_or_else(|| "af_sarah".into()),
        }),
        "piper" => Some(LocalVoice::Piper {
            voice: settings
                .speaker
                .clone()
                .unwrap_or_else(|| catalog::PIPER_VOICES[0].into()),
        }),
        _ => None,
    }
}

/// What these settings need downloaded.
fn packs(settings: &VoiceSettings) -> Result<Vec<Pack>, String> {
    let (runtime, _) = catalog::runtime().ok_or("voice isn't available on this platform yet")?;
    let hearing = Hearing::parse(&settings.hearing).unwrap_or(Hearing::Small);
    let mut packs = vec![runtime, catalog::vad(), hearing.pack()];
    match local_voice(settings) {
        Some(LocalVoice::Kokoro { .. }) => packs.push(catalog::kokoro()),
        Some(LocalVoice::Piper { voice }) => {
            packs.push(catalog::piper(&voice).ok_or(format!("no Piper voice `{voice}`"))?)
        }
        None => {}
    }
    if settings.wake_word {
        packs.push(catalog::wake_word());
    }
    Ok(packs)
}

/// Read the settings again, download what they need, and start (or restart) voice.
#[tauri::command]
pub async fn voice_prepare(
    app: AppHandle,
    voice: tauri::State<'_, Voice>,
) -> Result<Status, String> {
    let settings = settings();
    let mut status = Status {
        enabled: settings.enabled,
        read_typed: settings.read_typed,
        wake_word: settings.wake_word,
        ..Status::default()
    };
    // Off: stop listening for the wake word and leave everything downloaded alone.
    if !settings.enabled {
        if let Some(ear) = voice.ear.lock().unwrap().as_ref() {
            let _ = ear.send(EarCommand::Wake(false));
        }
        *voice.state.lock().unwrap() = status.clone();
        let _ = app.emit("voice-status", &status);
        return Ok(status);
    }
    let started = start(&app, &voice, &settings).await;
    match started {
        Ok(()) => status.ready = true,
        Err(err) => status.error = Some(err),
    }
    *voice.state.lock().unwrap() = status.clone();
    let _ = app.emit("voice-status", &status);
    Ok(status)
}

async fn start(app: &AppHandle, voice: &Voice, settings: &VoiceSettings) -> Result<(), String> {
    let base = voice_dir().ok_or("no data folder")?;
    let progress_app = app.clone();
    let progress = move |p: lucy_voice::Progress| {
        let _ = progress_app.emit("voice-progress", &p);
    };
    for pack in packs(settings)? {
        pack.install(&base, &progress)
            .await
            .map_err(|err| err.to_string())?;
    }
    let engine = Engine::load(&base).map_err(|err| err.to_string())?;
    let hearing = Hearing::parse(&settings.hearing).unwrap_or(Hearing::Small);

    // Hearing: a fresh thread with the new models (the old one ends when its channel drops).
    let (tx, rx) = mpsc::channel();
    let ear_engine = engine.clone();
    let ear_app = app.clone();
    let wake = settings.wake_word;
    let (loaded_tx, loaded_rx) = mpsc::channel();
    std::thread::spawn(move || {
        let parts = (|| -> Result<_, String> {
            let ear = Ear::new(&ear_engine, hearing).map_err(|e| e.to_string())?;
            let vad = Vad::new(&ear_engine, SILENCE_SECS).map_err(|e| e.to_string())?;
            let spotter = if wake {
                Some(WakeWord::new(&ear_engine).map_err(|e| e.to_string())?)
            } else {
                None
            };
            Ok((ear, vad, spotter))
        })();
        match parts {
            Ok((ear, vad, spotter)) => {
                let _ = loaded_tx.send(Ok(()));
                listen_loop(ear_app, rx, ear, vad, spotter, wake);
            }
            Err(err) => {
                let _ = loaded_tx.send(Err(err));
            }
        }
    });
    loaded_rx
        .recv()
        .map_err(|_| "the hearing thread stopped".to_string())??;
    *voice.ear.lock().unwrap() = Some(tx);

    // Speaking: a thread that turns queued sentences into speech.
    let speaker_voice = match local_voice(settings) {
        Some(local) => SpeakerVoice::Local(Mouth::new(&engine, &local).map_err(|e| e.to_string())?),
        None => SpeakerVoice::Hosted(hosted(settings)?),
    };
    let queue = Arc::clone(&voice.mouth);
    let mouth_app = app.clone();
    std::thread::spawn(move || speak_loop(mouth_app, queue, speaker_voice));
    Ok(())
}

/// A hosted voice, with its key from the keyring (`openai` or `elevenlabs`).
fn hosted(settings: &VoiceSettings) -> Result<Hosted, String> {
    let key_name = settings.voice.as_str();
    let key = SystemSecrets::system()
        .resolve(key_name)
        .map_err(|err| err.to_string())?
        .ok_or_else(|| format!("{key_name}'s voice needs its API key (Settings → Voice)"))?
        .value;
    Ok(match key_name {
        "openai" => Hosted::OpenAi {
            key,
            voice: settings.speaker.clone().unwrap_or_else(|| "nova".into()),
        },
        _ => Hosted::ElevenLabs {
            key,
            voice: settings
                .speaker
                .clone()
                .ok_or("choose an ElevenLabs voice ID in Settings → Voice")?,
        },
    })
}

enum SpeakerVoice {
    Local(Mouth),
    Hosted(Hosted),
}

fn listen_loop(
    app: AppHandle,
    commands: mpsc::Receiver<EarCommand>,
    mut ear: Ear,
    mut vad: Vad,
    mut spotter: Option<WakeWord>,
    mut wake: bool,
) {
    let (samples_tx, samples_rx) = mpsc::channel::<Vec<f32>>();
    let mut mic: Option<Mic> = None;
    // What you're saying, and who's listening (sprite or window) and how.
    let mut listening: Option<(bool, String)> = None;
    let mut heard: Vec<f32> = Vec::new();
    let open_mic = |mic: &mut Option<Mic>| {
        if mic.is_none() {
            let tx = samples_tx.clone();
            match Mic::open(move |chunk| {
                let _ = tx.send(chunk);
            }) {
                Ok(opened) => *mic = Some(opened),
                Err(err) => {
                    let _ = app.emit("voice-error", json!({"message": err.to_string()}));
                }
            }
        }
    };
    if wake && spotter.is_some() {
        open_mic(&mut mic);
    }
    loop {
        match commands.recv_timeout(Duration::from_millis(20)) {
            Ok(EarCommand::Listen { hold, origin }) => {
                heard.clear();
                vad.reset();
                listening = Some((hold, origin));
                open_mic(&mut mic);
                let _ = app.emit("voice-state", "listening");
            }
            Ok(EarCommand::Stop) => {
                if let Some((_, origin)) = listening.take() {
                    finish(&app, &mut ear, &heard, &origin);
                }
            }
            Ok(EarCommand::Wake(on)) => wake = on,
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }
        while let Ok(chunk) = samples_rx.try_recv() {
            if let Some((hold, origin)) = &listening {
                heard.extend_from_slice(&chunk);
                if !hold && vad.finished(&chunk) {
                    let origin = origin.clone();
                    listening = None;
                    finish(&app, &mut ear, &heard, &origin);
                }
            } else if wake
                && let Some(spotter) = spotter.as_mut()
                && spotter.heard(&chunk)
            {
                let _ = app.emit("voice-wake", ());
            }
        }
        // Nothing needs the microphone: close it.
        if listening.is_none() && !(wake && spotter.is_some()) {
            mic = None;
        }
    }
}

fn finish(app: &AppHandle, ear: &mut Ear, heard: &[f32], origin: &str) {
    let _ = app.emit("voice-state", "thinking");
    let text = ear.transcribe(heard);
    let _ = app.emit("voice-heard", json!({"origin": origin, "text": text}));
    let _ = app.emit("voice-state", "idle");
}

fn speak_loop(app: AppHandle, queue: Arc<MouthQueue>, mut voice: SpeakerVoice) {
    let speaker = match Speaker::open() {
        Ok(speaker) => speaker,
        Err(err) => {
            let _ = app.emit("voice-error", json!({"message": err.to_string()}));
            return;
        }
    };
    *queue.speaker.lock().unwrap() = Some(speaker.handle());
    let client = reqwest::Client::new();
    loop {
        let (generation, text) = {
            let mut pending = queue.pending.lock().unwrap();
            loop {
                if let Some(next) = pending.1.pop_front() {
                    break next;
                }
                pending = queue.ready.wait(pending).unwrap();
            }
        };
        let audio = match &mut voice {
            SpeakerVoice::Local(mouth) => Ok(mouth.speak(&text)),
            SpeakerVoice::Hosted(hosted) => {
                tauri::async_runtime::block_on(hosted.speak(&client, &text))
            }
        };
        // Hushed while it was being made: drop it.
        if queue.pending.lock().unwrap().0 != generation {
            continue;
        }
        match audio {
            Ok(audio) => speaker.play(&audio),
            Err(err) => {
                let _ = app.emit("voice-error", json!({"message": err.to_string()}));
            }
        }
    }
}

#[tauri::command]
pub fn voice_status(voice: tauri::State<'_, Voice>) -> Status {
    voice.state.lock().unwrap().clone()
}

/// Start listening: `hold` until `voice_stop_listening`, otherwise until you go quiet.
#[tauri::command]
pub fn voice_listen(
    voice: tauri::State<'_, Voice>,
    hold: bool,
    origin: String,
) -> Result<(), String> {
    let ear = voice.ear.lock().unwrap();
    let ear = ear.as_ref().ok_or("voice isn't on (Settings → Voice)")?;
    ear.send(EarCommand::Listen { hold, origin })
        .map_err(|_| "voice stopped; turn it on again in Settings → Voice".to_string())
}

#[tauri::command]
pub fn voice_stop_listening(voice: tauri::State<'_, Voice>) {
    if let Some(ear) = voice.ear.lock().unwrap().as_ref() {
        let _ = ear.send(EarCommand::Stop);
    }
}

/// Say `text` after whatever is queued.
#[tauri::command]
pub fn voice_say(voice: tauri::State<'_, Voice>, text: String) {
    let mut pending = voice.mouth.pending.lock().unwrap();
    let generation = pending.0;
    pending.1.push_back((generation, text));
    voice.mouth.ready.notify_one();
}

/// Stop talking now.
#[tauri::command]
pub fn voice_hush(voice: tauri::State<'_, Voice>) {
    let mut pending = voice.mouth.pending.lock().unwrap();
    pending.0 += 1;
    pending.1.clear();
    if let Some(speaker) = voice.mouth.speaker.lock().unwrap().as_ref() {
        speaker.stop();
    }
}

/// Start voice at launch if it's on (downloads, if any, happen in the background).
pub fn start_in_background(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let voice = app.state::<Voice>();
        if let Err(err) = voice_prepare(app.clone(), voice).await {
            eprintln!("lucy-desktop: voice: {err}");
        }
    });
}
