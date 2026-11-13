//! Downloading what voice needs the first time it's turned on: sherpa-onnx's library and the
//! models, from sherpa-onnx's GitHub releases, unpacked into Lucy's data folder.

use std::path::{Component, Path, PathBuf};

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
    /// Bytes to download: for progress, and a little past it the download is stopped.
    pub size: u64,
    /// SHA-256 of the download: anything else (a swapped or re-uploaded file) is refused.
    pub sha256: &'static str,
}

/// How many times its download size an archive may unpack to.
const MAX_EXPANSION: u64 = 8;

/// How far past its expected size a download may run before it's stopped.
fn size_limit(size: u64) -> u64 {
    size + size / 4 + 1024 * 1024
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
        let limit = size_limit(self.size);
        let too_big = || {
            VoiceError::Download(
                self.label.into(),
                "the download is much bigger than expected, so it was stopped".into(),
            )
        };
        if response
            .content_length()
            .is_some_and(|length| length > limit)
        {
            return Err(too_big());
        }
        let total = response.content_length().unwrap_or(self.size);
        let mut file = std::fs::File::create(&partial)?;
        let mut hash = Sha256::new();
        let mut done = 0;
        let mut body = response.bytes_stream();
        let mut last_report = 0;
        while let Some(chunk) = body.next().await {
            let chunk =
                chunk.map_err(|err| VoiceError::Download(self.label.into(), err.to_string()))?;
            done += chunk.len() as u64;
            if done > limit {
                drop(file);
                let _ = std::fs::remove_file(&partial);
                return Err(too_big());
            }
            std::io::Write::write_all(&mut file, &chunk)?;
            hash.update(&chunk);
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
        let got: String = hash.finalize().iter().map(|b| format!("{b:02x}")).collect();
        if got != self.sha256 {
            let _ = std::fs::remove_file(&partial);
            return Err(VoiceError::Download(
                self.label.into(),
                "the download isn't the file Lucy expects (its checksum differs): it may have \
                 been damaged on the way or changed where it's published; try again, and if it \
                 keeps happening, update Lucy"
                    .into(),
            ));
        }
        let base = base.to_owned();
        let installs = self.installs;
        let archive = self.url.ends_with(".tar.bz2");
        let size = self.size;
        tokio::task::spawn_blocking(move || unpack(&partial, &base, installs, archive, size))
            .await
            .map_err(|err| VoiceError::Download(installs.into(), err.to_string()))?
    }
}

/// Unpack an archive (or move a single file) into place, all or nothing. Hard links, links
/// leading out of the archive, and more than [`MAX_EXPANSION`] times the download unpacked
/// are refused.
fn unpack(
    partial: &Path,
    base: &Path,
    installs: &str,
    archive: bool,
    size: u64,
) -> Result<(), VoiceError> {
    let target = base.join(installs);
    if !archive {
        std::fs::rename(partial, &target)?;
        return Ok(());
    }
    // Unpack beside the target, then move the one top-level folder into place.
    let staging = base.join(format!(".{installs}.unpacking"));
    let _ = std::fs::remove_dir_all(&staging);
    std::fs::create_dir_all(&staging)?;
    let refuse = |why: &str| VoiceError::Download(installs.into(), format!("the archive {why}"));
    let reader = bzip2::read::BzDecoder::new(std::fs::File::open(partial)?);
    let mut archive = tar::Archive::new(reader);
    let mut unpacked_bytes = 0u64;
    for entry in archive.entries()? {
        let mut entry = entry?;
        let kind = entry.header().entry_type();
        if kind.is_hard_link() {
            return Err(refuse("has a hard link in it"));
        }
        if kind.is_symlink() {
            let leads_out = entry.link_name()?.is_none_or(|to| {
                to.is_absolute() || to.components().any(|c| matches!(c, Component::ParentDir))
            });
            if leads_out {
                return Err(refuse("has a link leading outside it"));
            }
        }
        unpacked_bytes += entry.header().size()?;
        if unpacked_bytes > size.saturating_mul(MAX_EXPANSION) {
            return Err(refuse("unpacks to far more than its size"));
        }
        // Refuses paths leading outside the staging folder.
        entry.unpack_in(&staging)?;
    }
    let top: Vec<PathBuf> = std::fs::read_dir(&staging)?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .collect();
    // A real folder, not a link to one.
    let unpacked = match top.as_slice() {
        [one] if std::fs::symlink_metadata(one).is_ok_and(|m| m.is_dir()) => one.clone(),
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

#[cfg(test)]
mod tests {
    use super::unpack;

    /// A `.tar.bz2` built by `add`, written to `dir/pack.part`.
    fn archive(
        dir: &std::path::Path,
        add: impl FnOnce(&mut tar::Builder<Vec<u8>>),
    ) -> std::path::PathBuf {
        let mut builder = tar::Builder::new(Vec::new());
        add(&mut builder);
        let tar = builder.into_inner().unwrap();
        let mut packed = bzip2::write::BzEncoder::new(Vec::new(), bzip2::Compression::default());
        std::io::Write::write_all(&mut packed, &tar).unwrap();
        let path = dir.join("pack.part");
        std::fs::write(&path, packed.finish().unwrap()).unwrap();
        path
    }

    fn file(builder: &mut tar::Builder<Vec<u8>>, path: &str, bytes: &[u8]) {
        let mut header = tar::Header::new_gnu();
        header.set_size(bytes.len() as u64);
        header.set_mode(0o644);
        builder.append_data(&mut header, path, bytes).unwrap();
    }

    fn link(builder: &mut tar::Builder<Vec<u8>>, path: &str, to: &str, kind: tar::EntryType) {
        let mut header = tar::Header::new_gnu();
        header.set_entry_type(kind);
        header.set_size(0);
        builder.append_link(&mut header, path, to).unwrap();
    }

    #[test]
    fn a_model_unpacks_into_place() {
        let dir = tempfile::tempdir().unwrap();
        let part = archive(dir.path(), |b| file(b, "model/tokens.txt", b"hi"));

        unpack(&part, dir.path(), "model", true, 1000).unwrap();

        assert_eq!(
            std::fs::read(dir.path().join("model/tokens.txt")).unwrap(),
            b"hi"
        );
    }

    #[test]
    fn links_out_of_the_archive_and_hard_links_are_refused() {
        for (to, kind) in [
            ("/etc/passwd", tar::EntryType::Symlink),
            ("../../outside", tar::EntryType::Symlink),
            ("model/a", tar::EntryType::Link),
        ] {
            let dir = tempfile::tempdir().unwrap();
            let part = archive(dir.path(), |b| {
                file(b, "model/a", b"x");
                link(b, "model/b", to, kind);
            });

            assert!(
                unpack(&part, dir.path(), "model", true, 1000).is_err(),
                "{to}"
            );
            assert!(!dir.path().join("model").exists(), "{to}");
        }
    }

    #[test]
    fn an_archive_that_unpacks_to_far_more_than_its_size_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let part = archive(dir.path(), |b| file(b, "model/big", &vec![0u8; 100_000]));

        assert!(unpack(&part, dir.path(), "model", true, 1_000).is_err());
    }
}
