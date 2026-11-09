//! Pictures in chats. What you attach is saved in `<data dir>/images/`, named by a hash of its
//! bytes, and chats keep only a reference. When a reply is built, models that can see get the
//! picture itself; for others, a vision helper (`[images] vision_profile`) describes it once,
//! and the description (cached next to the picture) goes into the message instead.

use std::path::{Path, PathBuf};

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use futures::StreamExt;
use lucy_protocol::rpc::{ImageData, ImageRef};
use lucy_protocol::{Image, Message, Role};
use lucy_providers::{ChatRequest, Provider, StreamEvent};
use sha2::{Digest, Sha256};

/// The largest picture accepted, decoded (most providers stop around here).
const MAX_BYTES: usize = 20 * 1024 * 1024;

/// What a model that can't see is told when there's no helper to describe the picture.
const CANT_SEE: &str = "[You were sent a picture, but this model can't see pictures. Say so, and \
                        suggest switching to a model that can, or setting a vision helper in \
                        Settings.]";

const DESCRIBE: &str = "Describe this picture for someone who can't see it, so they can answer \
                        questions about it: what it shows, any text in it (word for word), \
                        layout, colours and anything notable. Plain text, no preamble.";

/// File extensions for the picture types Lucy accepts.
fn extension(media_type: &str) -> Option<&'static str> {
    match media_type {
        "image/png" => Some("png"),
        "image/jpeg" => Some("jpg"),
        "image/gif" => Some("gif"),
        "image/webp" => Some("webp"),
        _ => None,
    }
}

fn media_type(id: &str) -> Option<&'static str> {
    match id.rsplit_once('.')?.1 {
        "png" => Some("image/png"),
        "jpg" => Some("image/jpeg"),
        "gif" => Some("image/gif"),
        "webp" => Some("image/webp"),
        _ => None,
    }
}

/// Save a picture you sent and return its reference.
pub fn save(dir: &Path, picture: &ImageData) -> Result<ImageRef, String> {
    let ext = extension(&picture.media_type).ok_or_else(|| {
        format!(
            "pictures must be PNG, JPEG, GIF or WebP, not {}",
            picture.media_type
        )
    })?;
    let bytes = STANDARD
        .decode(picture.data.as_bytes())
        .map_err(|err| format!("that picture isn't valid base64: {err}"))?;
    if bytes.len() > MAX_BYTES {
        return Err(format!(
            "that picture is {} MB; the limit is {} MB",
            bytes.len() / (1024 * 1024),
            MAX_BYTES / (1024 * 1024)
        ));
    }
    let hash = Sha256::digest(&bytes);
    let id = format!("{}.{ext}", hex(&hash[..16]));
    let path = dir.join(&id);
    if !path.exists() {
        std::fs::create_dir_all(dir).map_err(|err| format!("couldn't keep the picture: {err}"))?;
        std::fs::write(&path, &bytes).map_err(|err| format!("couldn't keep the picture: {err}"))?;
    }
    Ok(ImageRef {
        id,
        media_type: picture.media_type.clone(),
        made: false,
    })
}

/// A saved picture, by its id.
pub fn load(dir: &Path, id: &str) -> Result<ImageData, String> {
    let path = file(dir, id)?;
    let bytes = std::fs::read(&path).map_err(|_| format!("no picture `{id}`"))?;
    Ok(ImageData {
        media_type: media_type(id).unwrap_or("application/octet-stream").into(),
        data: STANDARD.encode(bytes),
    })
}

/// The file for `id`, refusing anything that isn't a name Lucy gave a picture.
fn file(dir: &Path, id: &str) -> Result<PathBuf, String> {
    let (name, _) = id
        .rsplit_once('.')
        .ok_or_else(|| format!("no picture `{id}`"))?;
    let valid =
        !name.is_empty() && name.chars().all(|c| c.is_ascii_hexdigit()) && media_type(id).is_some();
    if !valid {
        return Err(format!("no picture `{id}`"));
    }
    Ok(dir.join(id))
}

fn description_file(dir: &Path, id: &str) -> Option<PathBuf> {
    file(dir, id)
        .ok()
        .map(|path| path.with_extension("description.txt"))
}

/// Text standing in for a picture until the helper has described it.
fn placeholder(id: &str) -> String {
    format!("\u{1}picture:{id}\u{1}")
}

fn described(description: &str) -> String {
    format!("[Picture: {description}]")
}

/// How the model this reply uses gets pictures.
pub enum Sight {
    /// It can see: send the pictures.
    Sees,
    /// It can't, but a helper describes them.
    Helped,
    /// It can't, and nothing can describe them.
    Blind,
}

/// Turn a saved message into what the model gets, given its sight. Pictures the helper hasn't
/// described yet get a placeholder and are added to `pending`. Pictures she made are only
/// sent to a model that can see; the others already know what they asked for.
pub fn attach(
    dir: &Path,
    mut message: Message,
    images: &[ImageRef],
    sight: &Sight,
    pending: &mut Vec<ImageRef>,
) -> Message {
    let shown: Vec<ImageRef>;
    let images = match sight {
        Sight::Sees => images,
        Sight::Helped | Sight::Blind => {
            shown = images.iter().filter(|i| !i.made).cloned().collect();
            &shown[..]
        }
    };
    if images.is_empty() {
        return message;
    }
    match sight {
        Sight::Sees => {
            for image in images {
                match load(dir, &image.id) {
                    Ok(data) => message.images.push(Image {
                        media_type: data.media_type,
                        data: data.data,
                    }),
                    Err(_) => message.content += "\n\n[A picture that's no longer available.]",
                }
            }
        }
        Sight::Helped => {
            for image in images {
                let cached = description_file(dir, &image.id)
                    .and_then(|path| std::fs::read_to_string(path).ok());
                message.content += "\n\n";
                match cached {
                    Some(description) => message.content += &described(description.trim()),
                    None => {
                        message.content += &placeholder(&image.id);
                        if !pending.contains(image) {
                            pending.push(image.clone());
                        }
                    }
                }
            }
        }
        Sight::Blind => {
            message.content += "\n\n";
            message.content += CANT_SEE;
        }
    }
    message
}

/// Have `helper` describe each pending picture (caching the description), and put the
/// descriptions in place of their placeholders in `messages`.
pub async fn describe_pending(
    dir: &Path,
    helper: &dyn Provider,
    pending: &[ImageRef],
    messages: &mut [Message],
) {
    for image in pending {
        let text = match describe(dir, helper, image).await {
            Ok(description) => {
                if let Some(path) = description_file(dir, &image.id) {
                    let _ = std::fs::write(path, &description);
                }
                described(&description)
            }
            Err(err) => format!("[A picture the vision helper couldn't describe: {err}]"),
        };
        let mark = placeholder(&image.id);
        for message in messages.iter_mut() {
            if message.content.contains(&mark) {
                message.content = message.content.replace(&mark, &text);
            }
        }
    }
}

async fn describe(dir: &Path, helper: &dyn Provider, image: &ImageRef) -> Result<String, String> {
    let data = load(dir, &image.id)?;
    let request = ChatRequest {
        messages: vec![
            Message::new(Role::System, DESCRIBE),
            Message::user_with_images(
                "Describe this picture.",
                vec![Image {
                    media_type: data.media_type,
                    data: data.data,
                }],
            ),
        ],
        tools: Vec::new(),
    };
    let mut stream = helper
        .stream(request)
        .await
        .map_err(|err| err.to_string())?;
    let mut text = String::new();
    while let Some(event) = stream.next().await {
        if let StreamEvent::TextDelta(delta) = event.map_err(|err| err.to_string())? {
            text.push_str(&delta);
        }
    }
    let text = text.trim();
    if text.is_empty() {
        return Err("it said nothing".into());
    }
    Ok(text.to_owned())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
