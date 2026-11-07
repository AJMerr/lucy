//! The configured sprite pack, handed to the page with its frames as data URLs.

use std::collections::BTreeMap;
use std::path::PathBuf;

use base64::Engine;
use lucy_core::config::{self, Config, SpriteSettings};
use lucy_packs::{Anchor, Pack};
use serde::Serialize;

/// The pack in use and how big to draw it.
pub struct Loaded {
    pack: Pack,
    scale: f64,
}

impl Loaded {
    /// The window's size in logical pixels: the pack's frame size times `[sprite] scale`.
    pub fn window_size(&self) -> (f64, f64) {
        let (width, height) = self.pack.size;
        (
            f64::from(width) * self.scale,
            f64::from(height) * self.scale,
        )
    }
}

/// `[sprite] pack` from the config, from `<data dir>/packs` or built in. A pack that won't load
/// is reported and replaced by the placeholder, so Lucy still shows up.
pub fn load_configured() -> Loaded {
    let settings = config::default_path()
        .and_then(|path| match Config::load(&path) {
            Ok(config) => config,
            Err(err) => {
                eprintln!("lucy-sprite: {err}; using the default sprite settings");
                None
            }
        })
        .map(|config| config.sprite)
        .unwrap_or_default();
    let SpriteSettings { pack, scale, .. } = settings;
    let pack = lucy_packs::load(&pack, packs_dir().as_deref()).unwrap_or_else(|err| {
        eprintln!("lucy-sprite: {err}; showing the placeholder instead");
        lucy_packs::load("placeholder", None).expect("the placeholder pack is built in")
    });
    Loaded {
        pack,
        scale: if scale > 0.0 && scale.is_finite() {
            scale
        } else {
            1.0
        },
    }
}

/// `<data dir>/packs`, next to the chats database (`LUCY_DATA_DIR` moves both).
fn packs_dir() -> Option<PathBuf> {
    lucy_core::sessions::default_path().and_then(|db| db.parent().map(|dir| dir.join("packs")))
}

#[derive(Serialize)]
pub struct PackView {
    name: String,
    width: u32,
    height: u32,
    scale: f64,
    anchor: &'static str,
    states: BTreeMap<String, StateView>,
}

#[derive(Serialize)]
struct StateView {
    /// `data:image/png;base64,…`
    frames: Vec<String>,
    fps: f64,
    #[serde(rename = "loop")]
    looping: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    then: Option<String>,
}

/// The page asks for its pack once, when it loads.
#[tauri::command]
pub fn pack(loaded: tauri::State<'_, Loaded>) -> PackView {
    let pack = &loaded.pack;
    let states = pack
        .states
        .iter()
        .map(|(name, state)| {
            let frames = state
                .frames
                .iter()
                .map(|frame| {
                    let png = base64::engine::general_purpose::STANDARD.encode(&frame.png);
                    format!("data:image/png;base64,{png}")
                })
                .collect();
            let view = StateView {
                frames,
                fps: state.fps,
                looping: state.looping,
                then: state.then.clone(),
            };
            (name.clone(), view)
        })
        .collect();
    PackView {
        name: pack.name.clone(),
        width: pack.size.0,
        height: pack.size.1,
        scale: loaded.scale,
        anchor: match pack.anchor {
            Anchor::BottomCenter => "bottom-center",
            Anchor::BottomLeft => "bottom-left",
            Anchor::BottomRight => "bottom-right",
            Anchor::Center => "center",
        },
        states,
    }
}
