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
    // Developer and package-manager credentials.
    "~/.config/gh/**",
    "~/.config/hub",
    "~/.npmrc",
    "~/.yarnrc",
    "~/.yarnrc.yml",
    "~/.pypirc",
    "~/.cargo/credentials",
    "~/.cargo/credentials.toml",
    "~/.gem/credentials",
    "~/.m2/settings.xml",
    "~/.gradle/gradle.properties",
    "~/.config/rclone/**",
    "~/.s3cfg",
    "~/.boto",
    "~/.terraform.d/**",
    "~/.vault-token",
    "~/.pgpass",
    "~/.my.cnf",
    "~/.config/sops/**",
    "~/.config/op/**",
    "~/.local/share/kwalletd/**",
    "~/.config/Bitwarden*/**",
    // AI tools' own credentials and history.
    "~/.claude/**",
    "~/.claude.json",
    "~/.codex/**",
    "~/.config/github-copilot/**",
    // Browser, mail and messenger profiles.
    "~/.mozilla/**",
    "~/.config/google-chrome/**",
    "~/.config/chromium/**",
    "~/.config/BraveSoftware/**",
    "~/.thunderbird/**",
    "~/.config/Signal/**",
    "~/Library/Application Support/Google/Chrome/**",
    "~/Library/Application Support/Firefox/**",
    "~/Library/Keychains/**",
    // Shell histories.
    "~/.bash_history",
    "~/.zsh_history",
    "~/.local/share/fish/fish_history",
    "~/.python_history",
    "~/.node_repl_history",
    "~/.psql_history",
    // Lucy's own chats, memory and remote access settings.
    "~/.local/share/lucy/**",
    "~/.config/lucy/remote.toml",
    "**/.env",
    "**/.env.*",
    "**/*.pem",
    "**/*.key",
    "**/id_rsa*",
    "**/id_ecdsa*",
    "**/id_ed25519*",
    "**/*.p12",
    "**/*.pfx",
    "**/*.ppk",
    "**/*.kdbx",
    "**/*.gpg",
    "**/*.asc",
    "**/*.tfstate",
    "**/*.tfstate.backup",
    "**/credentials",
    "**/credentials.json",
    "**/*service-account*.json",
    "**/kubeconfig*",
];

/// Files that run code or change how Lucy behaves: shell startup files, autostart and
/// user services, `~/.local/bin`, git hooks, direnv and editor tasks. Changing one always
/// asks, at every level (Lucy's own config and data folders are added to these).
pub const DEFAULT_PROTECTED_PATHS: &[&str] = &[
    "~/.bashrc",
    "~/.bash_profile",
    "~/.bash_login",
    "~/.bash_logout",
    "~/.profile",
    "~/.zshrc",
    "~/.zshenv",
    "~/.zprofile",
    "~/.zlogin",
    "~/.config/fish/**",
    "~/.config/nushell/**",
    "~/.config/environment.d/**",
    "~/.config/autostart/**",
    "~/.config/systemd/**",
    "~/.local/bin/**",
    "~/.local/share/applications/**",
    "~/.xprofile",
    "~/.xinitrc",
    "~/Library/LaunchAgents/**",
    "~/.ssh/**",
    "~/.gitconfig",
    "~/.config/git/**",
    "**/.git/hooks/**",
    "**/.git/config",
    "**/.envrc",
    "**/.vscode/tasks.json",
    "**/.vscode/settings.json",
    "**/.idea/workspace.xml",
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

    /// The built-in protected list ([`DEFAULT_PROTECTED_PATHS`]) plus `files`: a path ending in
    /// `/` covers everything in that folder.
    pub fn protected(home: Option<&Path>, files: &[PathBuf]) -> Self {
        let mut patterns: Vec<String> = DEFAULT_PROTECTED_PATHS
            .iter()
            .map(|p| (*p).to_owned())
            .collect();
        for file in files {
            let text = slashes(file);
            let escaped = globset::escape(text.trim_end_matches('/'));
            patterns.push(match text.ends_with('/') {
                true => format!("{escaped}/**"),
                false => escaped,
            });
        }
        Self::new(&patterns, home).unwrap_or_else(|_| {
            Self::new(DEFAULT_PROTECTED_PATHS, home).expect("built-in patterns are valid")
        })
    }

    /// Patterns starting with `~/` are skipped when there is no home folder.
    pub fn new(patterns: &[impl AsRef<str>], home: Option<&Path>) -> Result<Self, globset::Error> {
        // Home as given and, when it differs, its real path (on macOS `/var` is really
        // `/private/var`), so a path matches however it's spelled.
        let mut homes: Vec<String> = home.map(slashes).into_iter().collect();
        if let Some(real) = home
            .and_then(|home| home.canonicalize().ok())
            .map(|p| slashes(&p))
            && !homes.contains(&real)
        {
            homes.push(real);
        }
        let mut builder = GlobSetBuilder::new();
        for pattern in patterns {
            let pattern = pattern.as_ref();
            let expanded: Vec<String> = match pattern.strip_prefix("~/") {
                Some(rest) => homes
                    .iter()
                    .map(|home| format!("{}/{rest}", home.trim_end_matches('/')))
                    .collect(),
                // On Windows a pattern may be written with backslashes, or come from a resolved
                // `\\?\` path; elsewhere a backslash escapes the next character.
                None if cfg!(windows) => vec![slashes(Path::new(pattern))],
                None => vec![pattern.to_owned()],
            };
            for pattern in expanded {
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
