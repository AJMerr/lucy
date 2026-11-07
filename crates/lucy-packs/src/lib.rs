//! Sprite packs (ADR 0005): a folder with `pack.toml` and PNG frames for each state the
//! sprite can be in. Packs are found in the user's packs folder first, then among the ones
//! built into Lucy (only `placeholder`; no character art is committed).

use std::collections::BTreeMap;
use std::io;
use std::path::{Component, Path, PathBuf};

use serde::Deserialize;

/// States every pack must draw.
pub const REQUIRED_STATES: [&str; 3] = ["idle", "thinking", "talking"];
/// States a pack may draw; missing ones show `idle`.
pub const OPTIONAL_STATES: [&str; 3] = ["happy", "error", "listening"];
/// The packs built into Lucy.
pub const BUILT_IN: [&str; 1] = ["placeholder"];

/// The pack format this Lucy reads.
const FORMAT_VERSION: u32 = 1;
const PNG_SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";

/// A loaded, checked pack. `states` holds every state in [`REQUIRED_STATES`] and
/// [`OPTIONAL_STATES`]; optional ones the pack doesn't draw are copies of `idle`.
#[derive(Debug, Clone, PartialEq)]
pub struct Pack {
    pub name: String,
    pub author: Option<String>,
    /// Native frame size in pixels, before `[sprite] scale`.
    pub size: (u32, u32),
    /// The point that stays put on screen when the sprite is scaled.
    pub anchor: Anchor,
    pub states: BTreeMap<String, State>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct State {
    pub frames: Vec<Frame>,
    pub fps: f64,
    /// False plays the frames once, then switches to `then` (or stays on the last frame).
    pub looping: bool,
    pub then: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Frame {
    /// As written in `pack.toml`, relative to the pack folder.
    pub path: String,
    pub png: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Anchor {
    #[default]
    BottomCenter,
    BottomLeft,
    BottomRight,
    Center,
}

#[derive(Debug, thiserror::Error)]
pub enum PackError {
    #[error(
        "no sprite pack named `{name}`: looked in {} and among the built-in packs ({})",
        searched.as_ref().map_or_else(|| "no packs folder".to_owned(), |dir| dir.display().to_string()),
        BUILT_IN.join(", ")
    )]
    NotFound {
        name: String,
        searched: Option<PathBuf>,
    },
    #[error("couldn't read {path}: {source}")]
    Read {
        path: String,
        #[source]
        source: io::Error,
    },
    #[error("{path} isn't a valid pack manifest: {message}")]
    Manifest { path: String, message: String },
    /// The manifest parsed, but describes something Lucy can't draw.
    #[error("{manifest}: {problem}")]
    Invalid { manifest: String, problem: String },
}

/// Load the pack called `name`: `<user_dir>/<name>/pack.toml` if it exists, else a built-in
/// pack of that name.
pub fn load(name: &str, user_dir: Option<&Path>) -> Result<Pack, PackError> {
    let not_found = || PackError::NotFound {
        name: name.to_owned(),
        searched: user_dir.map(Path::to_owned),
    };
    // A name is one folder, never a path somewhere else.
    if name.is_empty() || !is_inside(Path::new(name)) || name.contains(['/', '\\']) {
        return Err(not_found());
    }
    if let Some(dir) = user_dir.map(|dir| dir.join(name))
        && dir.join("pack.toml").is_file()
    {
        return load_from(&Source::Dir(dir));
    }
    match name {
        "placeholder" => load_from(&Source::BuiltIn(PLACEHOLDER)),
        _ => Err(not_found()),
    }
}

/// The placeholder pack, compiled in so it's always there.
const PLACEHOLDER: &[(&str, &[u8])] = &[
    (
        "pack.toml",
        include_bytes!("../../../packs/placeholder/pack.toml"),
    ),
    (
        "frames/idle-0.png",
        include_bytes!("../../../packs/placeholder/frames/idle-0.png"),
    ),
    (
        "frames/idle-1.png",
        include_bytes!("../../../packs/placeholder/frames/idle-1.png"),
    ),
    (
        "frames/thinking-0.png",
        include_bytes!("../../../packs/placeholder/frames/thinking-0.png"),
    ),
    (
        "frames/thinking-1.png",
        include_bytes!("../../../packs/placeholder/frames/thinking-1.png"),
    ),
    (
        "frames/thinking-2.png",
        include_bytes!("../../../packs/placeholder/frames/thinking-2.png"),
    ),
    (
        "frames/talking-0.png",
        include_bytes!("../../../packs/placeholder/frames/talking-0.png"),
    ),
    (
        "frames/talking-1.png",
        include_bytes!("../../../packs/placeholder/frames/talking-1.png"),
    ),
    (
        "frames/happy-0.png",
        include_bytes!("../../../packs/placeholder/frames/happy-0.png"),
    ),
    (
        "frames/error-0.png",
        include_bytes!("../../../packs/placeholder/frames/error-0.png"),
    ),
];

/// Where a pack's files come from.
enum Source {
    Dir(PathBuf),
    BuiltIn(&'static [(&'static str, &'static [u8])]),
}

impl Source {
    fn read(&self, rel: &str) -> io::Result<Vec<u8>> {
        match self {
            Source::Dir(dir) => std::fs::read(dir.join(rel)),
            Source::BuiltIn(files) => files
                .iter()
                .find(|(path, _)| *path == rel)
                .map(|(_, bytes)| bytes.to_vec())
                .ok_or_else(|| io::ErrorKind::NotFound.into()),
        }
    }

    /// How to name a file in messages.
    fn describe(&self, rel: &str) -> String {
        match self {
            Source::Dir(dir) => dir.join(rel).display().to_string(),
            Source::BuiltIn(_) => format!("(built-in) {rel}"),
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    name: String,
    #[serde(default)]
    author: Option<String>,
    format_version: u32,
    kind: String,
    size: [u32; 2],
    #[serde(default)]
    anchor: Option<String>,
    states: BTreeMap<String, StateSpec>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StateSpec {
    frames: Vec<String>,
    fps: f64,
    #[serde(default = "looping_by_default", rename = "loop")]
    looping: bool,
    #[serde(default)]
    then: Option<String>,
}

fn looping_by_default() -> bool {
    true
}

fn load_from(source: &Source) -> Result<Pack, PackError> {
    let manifest_path = source.describe("pack.toml");
    let text = source.read("pack.toml").map_err(|source| PackError::Read {
        path: manifest_path.clone(),
        source,
    })?;
    let manifest: Manifest = std::str::from_utf8(&text)
        .map_err(|err| err.to_string())
        .and_then(|text| toml::from_str(text).map_err(|err| err.to_string()))
        .map_err(|message| PackError::Manifest {
            path: manifest_path.clone(),
            message,
        })?;
    let invalid = |problem: String| PackError::Invalid {
        manifest: manifest_path.clone(),
        problem,
    };

    if manifest.format_version != FORMAT_VERSION {
        return Err(invalid(format!(
            "format_version {} isn't supported (this Lucy reads {FORMAT_VERSION})",
            manifest.format_version
        )));
    }
    match manifest.kind.as_str() {
        "frames" => {}
        "live2d" => {
            return Err(invalid(
                "live2d packs aren't supported yet; use kind = \"frames\"".into(),
            ));
        }
        other => return Err(invalid(format!("unknown kind `{other}`; use \"frames\""))),
    }
    let [width, height] = manifest.size;
    if width == 0 || height == 0 {
        return Err(invalid("size must be at least 1×1 pixels".into()));
    }
    let anchor = match manifest.anchor.as_deref() {
        None | Some("bottom-center") => Anchor::BottomCenter,
        Some("bottom-left") => Anchor::BottomLeft,
        Some("bottom-right") => Anchor::BottomRight,
        Some("center") => Anchor::Center,
        Some(other) => {
            return Err(invalid(format!(
                "unknown anchor `{other}`; use bottom-center, bottom-left, bottom-right or center"
            )));
        }
    };

    let known: Vec<&str> = REQUIRED_STATES
        .iter()
        .chain(&OPTIONAL_STATES)
        .copied()
        .collect();
    for name in manifest.states.keys() {
        if !known.contains(&name.as_str()) {
            return Err(invalid(format!(
                "unknown state [states.{name}]; states are {}",
                known.join(", ")
            )));
        }
    }
    for required in REQUIRED_STATES {
        if !manifest.states.contains_key(required) {
            return Err(invalid(format!(
                "there's no [states.{required}]; {} are required",
                REQUIRED_STATES.join(", ")
            )));
        }
    }

    let mut states = BTreeMap::new();
    for (name, spec) in &manifest.states {
        let section = format!("[states.{name}]");
        if spec.frames.is_empty() {
            return Err(invalid(format!("{section} has no frames")));
        }
        if !(spec.fps > 0.0 && spec.fps.is_finite()) {
            return Err(invalid(format!("{section} needs fps above 0")));
        }
        if let Some(then) = &spec.then {
            if spec.looping {
                return Err(invalid(format!(
                    "{section} has `then`, but it loops; add `loop = false`"
                )));
            }
            if !known.contains(&then.as_str()) {
                return Err(invalid(format!(
                    "{section} says then = \"{then}\", but there's no such state"
                )));
            }
        }
        let mut frames = Vec::with_capacity(spec.frames.len());
        for path in &spec.frames {
            if !is_inside(Path::new(path)) {
                return Err(invalid(format!(
                    "frame `{path}` in {section} must be inside the pack folder"
                )));
            }
            let png = source.read(path).map_err(|err| {
                invalid(match err.kind() {
                    io::ErrorKind::NotFound => {
                        format!("frame `{path}` in {section} doesn't exist")
                    }
                    _ => format!("couldn't read frame `{path}` in {section}: {err}"),
                })
            })?;
            if !png.starts_with(PNG_SIGNATURE) {
                return Err(invalid(format!("frame `{path}` in {section} isn't a PNG")));
            }
            frames.push(Frame {
                path: path.clone(),
                png,
            });
        }
        states.insert(
            name.clone(),
            State {
                frames,
                fps: spec.fps,
                looping: spec.looping,
                then: spec.then.clone(),
            },
        );
    }
    let idle = states["idle"].clone();
    for optional in OPTIONAL_STATES {
        states
            .entry(optional.to_owned())
            .or_insert_with(|| idle.clone());
    }

    Ok(Pack {
        name: manifest.name,
        author: manifest.author,
        size: (width, height),
        anchor,
        states,
    })
}

/// A relative path that stays inside its folder: no root, drive or `..`.
fn is_inside(path: &Path) -> bool {
    path.components()
        .all(|component| matches!(component, Component::Normal(_) | Component::CurDir))
}
