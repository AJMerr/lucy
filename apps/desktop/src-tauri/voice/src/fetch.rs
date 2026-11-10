//! Downloading what voice needs the first time it's turned on: sherpa-onnx's library and the
//! models, from sherpa-onnx's GitHub releases, unpacked into Lucy's data folder.

use std::path::{Path, PathBuf};

use futures_util::StreamExt;
use sha2::{Digest, Sha256};

use crate::VoiceError;

/// Something to download: a `.tar.bz2` unpacked into the voice folder, or a single file.
#[derive(Debug, Clone, PartialEq)]
pub struct Pack {
    /// What it is, for progress ("Whisper small").
    pub label: &'static str,
    pub url: String,
    /// The folder (or file) it becomes under the voice folder; present means installed.
    pub installs: &'static str,
    /// Bytes to download, for progress.
    pub size: u64,
    /// SHA-256 of the download, when it's published.
    pub sha256: Option<&'static str>,
}

/// How far a download has got.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Progress {
    pub label: String,
    pub done: u64,
    pub total: u64,
}

impl Pack {
    pub fn installed(&self, base: &Path) -> bool {
        base.join(self.installs).exists()
    }

    /// Download and unpack into `base`, reporting progress; nothing to do if installed.
    pub async fn install(
        &self,
        base: &Path,
        progress: &(dyn Fn(Progress) + Send + Sync),
    ) -> Result<(), VoiceError> {
        if self.installed(base) {
            return Ok(());
        }
        tokio::fs::create_dir_all(base).await?;
        let partial = base.join(format!(".{}.part", self.installs.replace('/', "_")));
        let response = reqwest::get(&self.url)
            .await
            .and_then(reqwest::Response::error_for_status)
            .map_err(|err| VoiceError::Download(self.label.into(), err.to_string()))?;
        let total = response.content_length().unwrap_or(self.size);
        let mut file = std::fs::File::create(&partial)?;
        let mut hash = Sha256::new();
        let mut done = 0;
        let mut body = response.bytes_stream();
        let mut last_report = 0;
        while let Some(chunk) = body.next().await {
            let chunk =
                chunk.map_err(|err| VoiceError::Download(self.label.into(), err.to_string()))?;
            std::io::Write::write_all(&mut file, &chunk)?;
            hash.update(&chunk);
            done += chunk.len() as u64;
            // About every 1%, so a slow UI isn't flooded.
            if done - last_report > total / 100 || done == total {
                last_report = done;
                progress(Progress {
                    label: self.label.into(),
                    done,
                    total,
                });
            }
        }
        drop(file);
        if let Some(expected) = self.sha256 {
            let got: String = hash.finalize().iter().map(|b| format!("{b:02x}")).collect();
            if got != expected {
                let _ = std::fs::remove_file(&partial);
                return Err(VoiceError::Download(
                    self.label.into(),
                    "the download was corrupted (checksum mismatch)".into(),
                ));
            }
        }
        let base = base.to_owned();
        let installs = self.installs;
        let archive = self.url.ends_with(".tar.bz2");
        tokio::task::spawn_blocking(move || unpack(&partial, &base, installs, archive))
            .await
            .map_err(|err| VoiceError::Download(installs.into(), err.to_string()))?
    }
}

/// Unpack an archive (or move a single file) into place, all or nothing.
fn unpack(partial: &Path, base: &Path, installs: &str, archive: bool) -> Result<(), VoiceError> {
    let target = base.join(installs);
    if !archive {
        std::fs::rename(partial, &target)?;
        return Ok(());
    }
    // Unpack beside the target, then move the one top-level folder into place.
    let staging = base.join(format!(".{installs}.unpacking"));
    let _ = std::fs::remove_dir_all(&staging);
    std::fs::create_dir_all(&staging)?;
    let reader = bzip2::read::BzDecoder::new(std::fs::File::open(partial)?);
    tar::Archive::new(reader).unpack(&staging)?;
    let top: Vec<PathBuf> = std::fs::read_dir(&staging)?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .collect();
    let unpacked = match top.as_slice() {
        [one] if one.is_dir() => one.clone(),
        _ => staging.clone(),
    };
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::rename(&unpacked, &target)?;
    let _ = std::fs::remove_dir_all(&staging);
    let _ = std::fs::remove_file(partial);
    Ok(())
}
