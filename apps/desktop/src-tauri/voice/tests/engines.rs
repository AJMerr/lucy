//! The engines on real models: she hears what she says, notices when you stop talking, and
//! wakes to "Hey Lucy" (and not to other words). The clips are made with her own local voice.
//!
//! Downloads sherpa-onnx's library and about 230 MB of models into `target/voice-models` the
//! first time, and loads that library, so it only runs when asked:
//! `cargo test -p lucy-voice -- --ignored` (in apps/desktop/src-tauri, in `nix develop`).

use std::path::PathBuf;

use lucy_voice::audio::Resampler;
use lucy_voice::{Ear, Engine, Hearing, LocalVoice, Mouth, SAMPLE_RATE, Vad, WakeWord, catalog};

fn models() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../target/voice-models")
}

async fn install() -> Engine {
    let base = models();
    let packs = [
        catalog::runtime().expect("a runtime for this platform").0,
        catalog::piper("en_US-amy-medium").unwrap(),
        Hearing::Tiny.pack(),
        catalog::wake_word(),
        catalog::vad(),
    ];
    for pack in packs {
        pack.install(&base, &|_| {}).await.unwrap();
    }
    Engine::load(&base).unwrap()
}

/// `text` in her Piper voice, at 16 kHz, with half a second of quiet either side.
fn say(mouth: &mut Mouth, text: &str) -> Vec<f32> {
    let audio = mouth.speak(text);
    assert!(!audio.samples.is_empty(), "she said nothing for {text:?}");
    let mut clip = vec![0.0; SAMPLE_RATE as usize / 2];
    Resampler::new(audio.sample_rate, SAMPLE_RATE).push(&audio.samples, &mut clip);
    clip.extend(vec![0.0; SAMPLE_RATE as usize / 2]);
    clip
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "downloads sherpa-onnx and about 230 MB of models"]
async fn she_hears_what_she_says_and_wakes_to_her_name() {
    let engine = install().await;
    let mut mouth = Mouth::new(
        &engine,
        &LocalVoice::Piper {
            voice: "en_US-amy-medium".into(),
        },
    )
    .unwrap();
    let wake = say(&mut mouth, "Hey Lucy!");
    let other = say(&mut mouth, "The weather is lovely today.");

    let mut ear = Ear::new(&engine, Hearing::Tiny).unwrap();
    let heard = ear.transcribe(&other).to_lowercase();
    assert!(heard.contains("weather"), "heard {heard:?}");

    let mut vad = Vad::new(&engine, 0.4).unwrap();
    let ended = other.chunks(1600).any(|chunk| vad.finished(chunk));
    assert!(ended, "noticed the sentence end");

    let mut spotter = WakeWord::new(&engine).unwrap();
    let woke = wake.chunks(1600).any(|chunk| spotter.heard(chunk));
    assert!(woke, "woke to \"Hey Lucy\"");
    let mut spotter = WakeWord::new(&engine).unwrap();
    let woke = other.chunks(1600).any(|chunk| spotter.heard(chunk));
    assert!(!woke, "didn't wake to other words");
}
