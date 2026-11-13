//! Path resolution and the list of private paths Lucy never reads without asking.

use std::path::{Path, PathBuf};

use globset::{GlobBuilder, GlobSet, GlobSetBuilder};

/// Secrets and credentials. `~/` patterns are anchored at home; `**/` ones match anywhere.
pub const DEFAULT_PRIVATE_PATHS: &[&str] = &[
    "~/.ssh/**",
    "~/.gnupg/**",
    "~/.aws/**",
    "~/.azure/**",
    "~/.config/gcloud/**",
    "~/.kube/**",
    "~/.docker/config.json",
    "~/.netrc",
    "~/.git-credentials",
    "~/.password-store/**",
    "~/.local/share/keyrings/**",
    "~/.pki/**",
    "**/.env",
    "**/.env.*",
    "**/*.pem",
    "**/*.key",
    "**/id_rsa*",
    "**/id_ecdsa*",
    "**/id_ed25519*",
];

/// Paths matching any of a set of glob patterns.
#[derive(Clone, Debug)]
pub struct PrivatePaths {
    set: GlobSet,
}

impl PrivatePaths {
    /// The built-in list ([`DEFAULT_PRIVATE_PATHS`]).
    pub fn defaults(home: Option<&Path>) -> Self {
        Self::new(DEFAULT_PRIVATE_PATHS, home).expect("built-in patterns are valid")
    }

    /// Patterns starting with `~/` are skipped when there is no home folder.
    pub fn new(patterns: &[impl AsRef<str>], home: Option<&Path>) -> Result<Self, globset::Error> {
        let home = home.map(slashes);
        let mut builder = GlobSetBuilder::new();
        for pattern in patterns {
            let pattern = pattern.as_ref();
            let pattern = match (pattern.strip_prefix("~/"), &home) {
                (Some(rest), Some(home)) => format!("{}/{rest}", home.trim_end_matches('/')),
                (Some(_), None) => continue,
                (None, _) => pattern.to_owned(),
            };
            // `dir/**` should also cover `dir` itself (listing it reveals file names).
            let base = pattern.strip_suffix("/**").map(str::to_owned);
            for pattern in std::iter::once(pattern).chain(base) {
                builder.add(
                    GlobBuilder::new(&pattern)
                        .literal_separator(true)
                        .case_insensitive(cfg!(any(windows, target_os = "macos")))
                        .build()?,
                );
            }
        }
        Ok(Self {
            set: builder.build()?,
        })
    }

    pub fn contains(&self, path: &Path) -> bool {
        self.set.is_match(slashes(path))
    }
}

/// The path as a string with `/` separators, so one pattern works on every OS. Windows'
/// verbatim prefix (`\\?\C:\…`, from resolving a path) is dropped so it matches like `C:\…`.
fn slashes(path: &Path) -> String {
    let path = path.to_string_lossy().replace('\\', "/");
    if let Some(unc) = path.strip_prefix("//?/UNC/") {
        format!("//{unc}")
    } else if let Some(rest) = path.strip_prefix("//?/") {
        rest.to_owned()
    } else {
        path
    }
}

/// `path` made absolute: `~` is home, relative paths start at `cwd`.
pub fn resolve(path: &str, cwd: &Path) -> PathBuf {
    let home = dirs::home_dir();
    match (path, &home) {
        ("~", Some(home)) => home.clone(),
        (p, Some(home)) if p.starts_with("~/") || p.starts_with("~\\") => home.join(&p[2..]),
        (p, _) => cwd.join(p),
    }
}

/// How to show `path` to the model: relative to `cwd` when inside it.
pub fn display(path: &Path, cwd: &Path) -> String {
    path.strip_prefix(cwd)
        .ok()
        .filter(|rel| !rel.as_os_str().is_empty())
        .unwrap_or(path)
        .display()
        .to_string()
}
