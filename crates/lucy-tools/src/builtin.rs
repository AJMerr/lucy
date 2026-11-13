//! The built-in tools: files, search, the web and the shell. Each chat turn gets its own
//! in-process server rooted at the folder the user is working in.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, ContentBlock, ServerCapabilities, ServerConfig};
use rmcp::service::{ClientInitializeError, RunningService};
use rmcp::{RoleClient, ServerHandler, ServiceExt, tool, tool_handler, tool_router};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;
use tokio::io::AsyncReadExt;

pub use crate::paths::PrivatePaths;
use crate::paths::{display, resolve};

/// Lines `read_file` returns when no limit is given.
const READ_LINES: usize = 2000;
/// Cap on any single tool output, so one call can't flood the model's context.
const MAX_OUTPUT: usize = 100 * 1024;
/// The largest picture `read_file` shows (most providers stop around here).
const MAX_PICTURE: usize = 20 * 1024 * 1024;
const MAX_LISTING: usize = 1000;
const MAX_MATCHES: usize = 200;
const MAX_MATCH_LINE: usize = 300;
/// Files bigger than this aren't searched.
const MAX_SEARCH_FILE: u64 = 2 * 1024 * 1024;
const DEFAULT_COMMAND_TIMEOUT: u64 = 120;
const MAX_COMMAND_TIMEOUT: u64 = 600;
const FETCH_TIMEOUT: Duration = Duration::from_secs(30);
/// The most of a page read; past it, the rest is cut off.
const MAX_FETCH: usize = 5 * 1024 * 1024;

pub struct BuiltinOptions {
    /// The user's working folder: relative paths and commands start here.
    pub cwd: PathBuf,
    /// Never searched (reads of them are gated by the core's permission rules).
    pub private: PrivatePaths,
}

/// Start a built-in server in this process and connect a client to it.
pub async fn connect(
    options: BuiltinOptions,
) -> Result<RunningService<RoleClient, ()>, Box<ClientInitializeError>> {
    let (server_io, client_io) = tokio::io::duplex(64 * 1024);
    let server = Builtin::new(options);
    tokio::spawn(async move {
        if let Ok(running) = server.serve(server_io).await {
            let _ = running.waiting().await;
        }
    });
    ().serve(client_io).await.map_err(Box::new)
}

#[derive(Clone)]
pub struct Builtin {
    cwd: PathBuf,
    private: PrivatePaths,
    /// For public addresses: never connects to this machine or a private network, even when
    /// redirected or when a name resolves there.
    http: reqwest::Client,
    /// For an address that names this machine or a private network itself (asked about every
    /// time): redirects stay on that host.
    local_http: reqwest::Client,
    tool_router: ToolRouter<Self>,
}

#[derive(Deserialize, JsonSchema)]
struct ReadFileArgs {
    /// The file: absolute, relative to the working folder, or starting with `~/`.
    path: String,
    /// First line to return, counting from 1 (default 1).
    offset: Option<usize>,
    /// How many lines to return (default 2000).
    limit: Option<usize>,
}

#[derive(Deserialize, JsonSchema)]
struct ListDirArgs {
    /// The folder (default: the working folder).
    path: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
struct SearchFilesArgs {
    /// Regular expression to look for, e.g. `fn main` or `TODO|FIXME`.
    pattern: String,
    /// Folder to search (default: the working folder).
    path: Option<String>,
    /// Only search files matching this glob, e.g. `*.rs` or `src/**/*.ts`.
    glob: Option<String>,
    /// Ignore upper/lower case (default false).
    ignore_case: Option<bool>,
}

#[derive(Deserialize, JsonSchema)]
struct WriteFileArgs {
    /// The file to create or replace.
    path: String,
    /// The complete new contents.
    content: String,
}

#[derive(Deserialize, JsonSchema)]
struct EditFileArgs {
    /// The file to change.
    path: String,
    /// Exact text to replace, including whitespace. Must appear exactly once unless
    /// `replace_all` is set; include surrounding lines to make it unique.
    old_text: String,
    /// The replacement text.
    new_text: String,
    /// Replace every occurrence instead of exactly one (default false).
    replace_all: Option<bool>,
}

#[derive(Deserialize, JsonSchema)]
struct RunCommandArgs {
    /// Shell command line (`sh -c` on Linux and macOS, `cmd /C` on Windows).
    command: String,
    /// Stop the command after this many seconds (default 120, at most 600).
    timeout_secs: Option<u64>,
}

#[derive(Deserialize, JsonSchema)]
struct FetchUrlArgs {
    /// An http:// or https:// address.
    url: String,
}

#[tool_router]
impl Builtin {
    pub fn new(options: BuiltinOptions) -> Self {
        Self {
            cwd: options.cwd,
            private: options.private,
            http: reqwest::Client::builder()
                .timeout(FETCH_TIMEOUT)
                .user_agent(concat!("Lucy/", env!("CARGO_PKG_VERSION")))
                .dns_resolver(std::sync::Arc::new(PublicOnly))
                .redirect(reqwest::redirect::Policy::custom(|attempt| {
                    if attempt.previous().len() >= 10 {
                        attempt.error("too many redirects")
                    } else if names_private_host(attempt.url().as_str()) {
                        attempt.error("it redirects to this machine or a private network")
                    } else {
                        attempt.follow()
                    }
                }))
                .build()
                .expect("HTTP client builds"),
            local_http: reqwest::Client::builder()
                .timeout(FETCH_TIMEOUT)
                .user_agent(concat!("Lucy/", env!("CARGO_PKG_VERSION")))
                .redirect(reqwest::redirect::Policy::custom(|attempt| {
                    let same_host = attempt
                        .previous()
                        .first()
                        .is_some_and(|first| first.host_str() == attempt.url().host_str());
                    if attempt.previous().len() >= 10 || !same_host {
                        attempt.error("it redirects to another host")
                    } else {
                        attempt.follow()
                    }
                }))
                .build()
                .expect("HTTP client builds"),
            tool_router: Self::tool_router(),
        }
    }

    /// Read a text file, or look at a picture (PNG, JPEG, GIF or WebP). Long files come back a
    /// page at a time; the reply says how to read on.
    #[tool(annotations(read_only_hint = true, open_world_hint = false))]
    async fn read_file(
        &self,
        Parameters(args): Parameters<ReadFileArgs>,
    ) -> Result<CallToolResult, String> {
        let path = resolve(&args.path, &self.cwd);
        let bytes = tokio::fs::read(&path)
            .await
            .map_err(|err| format!("could not read {}: {err}", path.display()))?;
        if let Some(media_type) = picture_type(&bytes) {
            if bytes.len() > MAX_PICTURE {
                return Err(format!(
                    "{} is a picture over {} MB, too big to look at",
                    path.display(),
                    MAX_PICTURE / (1024 * 1024)
                ));
            }
            return Ok(CallToolResult::success(vec![
                ContentBlock::text(format!("The picture {}.", path.display())),
                ContentBlock::image(STANDARD.encode(&bytes), media_type),
            ]));
        }
        Ok(CallToolResult::success(vec![ContentBlock::text(
            read_text(&path, &bytes, &args)?,
        )]))
    }

    /// List a folder: subfolders first (ending in `/`), then files with their sizes.
    #[tool(annotations(read_only_hint = true, open_world_hint = false))]
    async fn list_dir(&self, Parameters(args): Parameters<ListDirArgs>) -> Result<String, String> {
        let path = resolve(args.path.as_deref().unwrap_or("."), &self.cwd);
        let mut entries = tokio::fs::read_dir(&path)
            .await
            .map_err(|err| format!("could not list {}: {err}", path.display()))?;
        let mut dirs = Vec::new();
        let mut files = Vec::new();
        while let Ok(Some(entry)) = entries.next_entry().await {
            let name = entry.file_name().to_string_lossy().into_owned();
            // Follow symlinks so a link to a folder lists as a folder.
            match tokio::fs::metadata(entry.path()).await {
                Ok(meta) if meta.is_dir() => dirs.push(format!("{name}/")),
                Ok(meta) => files.push(format!("{name}  ({})", size(meta.len()))),
                Err(_) => files.push(format!("{name}  (broken link)")),
            }
        }
        dirs.sort_by_key(|n| n.to_lowercase());
        files.sort_by_key(|n| n.to_lowercase());
        let total = dirs.len() + files.len();
        if total == 0 {
            return Ok("(empty folder)".into());
        }
        let mut listing: Vec<String> = dirs.into_iter().chain(files).take(MAX_LISTING).collect();
        if total > MAX_LISTING {
            listing.push(format!("[… and {} more]", total - MAX_LISTING));
        }
        Ok(listing.join("\n"))
    }

    /// Search file contents for a regular expression, like grep. Skips hidden, git-ignored,
    /// binary and private files. Replies with `path:line: text` for each match.
    #[tool(annotations(read_only_hint = true, open_world_hint = false))]
    async fn search_files(
        &self,
        Parameters(args): Parameters<SearchFilesArgs>,
    ) -> Result<String, String> {
        let regex = regex::RegexBuilder::new(&args.pattern)
            .case_insensitive(args.ignore_case.unwrap_or(false))
            .build()
            .map_err(|err| format!("bad pattern: {err}"))?;
        let glob = match &args.glob {
            Some(glob) => Some(
                globset::GlobBuilder::new(glob)
                    .literal_separator(glob.contains('/'))
                    .build()
                    .map_err(|err| format!("bad glob: {err}"))?
                    .compile_matcher(),
            ),
            None => None,
        };
        let given = resolve(args.path.as_deref().unwrap_or("."), &self.cwd);
        // Walk the real folder, so `~/./.config` or a link to a private folder is still
        // recognised as private; private paths are matched both ways.
        let root = Root {
            real: given
                .canonicalize()
                .map_err(|_| format!("{} does not exist", given.display()))?,
            given: given.components().collect(),
        };
        let cwd = self.cwd.canonicalize().unwrap_or_else(|_| self.cwd.clone());
        let private = self.private.clone();
        tokio::task::spawn_blocking(move || search(&root, &cwd, &regex, glob.as_ref(), &private))
            .await
            .map_err(|err| format!("search failed: {err}"))
    }

    /// Create or completely replace a text file (missing folders are created).
    #[tool(annotations(
        read_only_hint = false,
        destructive_hint = true,
        open_world_hint = false
    ))]
    async fn write_file(
        &self,
        Parameters(args): Parameters<WriteFileArgs>,
    ) -> Result<String, String> {
        let path = resolve(&args.path, &self.cwd);
        let existed = path.exists();
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|err| format!("could not create {}: {err}", parent.display()))?;
        }
        tokio::fs::write(&path, &args.content)
            .await
            .map_err(|err| format!("could not write {}: {err}", path.display()))?;
        Ok(format!(
            "{} {} ({})",
            if existed { "Replaced" } else { "Created" },
            display(&path, &self.cwd),
            size(args.content.len() as u64)
        ))
    }

    /// Change part of a text file by replacing exact text. Prefer this to rewriting whole
    /// files.
    #[tool(annotations(
        read_only_hint = false,
        destructive_hint = true,
        open_world_hint = false
    ))]
    async fn edit_file(
        &self,
        Parameters(args): Parameters<EditFileArgs>,
    ) -> Result<String, String> {
        let path = resolve(&args.path, &self.cwd);
        let shown = display(&path, &self.cwd);
        let text = tokio::fs::read_to_string(&path)
            .await
            .map_err(|err| format!("could not read {}: {err}", path.display()))?;
        let new = apply_edit(&text, &args, &shown)?;
        tokio::fs::write(&path, new)
            .await
            .map_err(|err| format!("could not write {}: {err}", path.display()))?;
        Ok(format!("Edited {shown}"))
    }

    /// Run a shell command in the working folder and return its output and exit code.
    #[tool(annotations(
        read_only_hint = false,
        destructive_hint = true,
        open_world_hint = true
    ))]
    async fn run_command(
        &self,
        Parameters(args): Parameters<RunCommandArgs>,
    ) -> Result<String, String> {
        let timeout = args
            .timeout_secs
            .unwrap_or(DEFAULT_COMMAND_TIMEOUT)
            .clamp(1, MAX_COMMAND_TIMEOUT);
        run(&args.command, &self.cwd, Duration::from_secs(timeout)).await
    }

    /// Fetch a web page or file over HTTP(S) and return it as text (HTML is converted).
    #[tool(annotations(read_only_hint = true, open_world_hint = true))]
    async fn fetch_url(
        &self,
        Parameters(args): Parameters<FetchUrlArgs>,
    ) -> Result<String, String> {
        if !(args.url.starts_with("http://") || args.url.starts_with("https://")) {
            return Err("only http:// and https:// addresses can be fetched".into());
        }
        let client = match names_private_host(&args.url) {
            true => &self.local_http,
            false => &self.http,
        };
        let mut response = client
            .get(&args.url)
            .send()
            .await
            .map_err(|err| format!("could not fetch {}: {err}", args.url))?;
        let status = response.status();
        let is_html = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.contains("html"));
        // Read only so much: a huge (or endless) page mustn't fill memory.
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|err| format!("could not read {}: {err}", args.url))?
        {
            bytes.extend_from_slice(&chunk);
            if bytes.len() >= MAX_FETCH {
                bytes.truncate(MAX_FETCH);
                break;
            }
        }
        let body = String::from_utf8_lossy(&bytes).into_owned();
        let text = if is_html {
            crate::html::to_text(&body)
        } else {
            body
        };
        if !status.is_success() {
            return Err(format!("HTTP {status}\n{}", truncate(&text, 2000)));
        }
        Ok(truncate(&text, MAX_OUTPUT))
    }
}

/// Variables in Lucy's environment holding keys she uses (`LUCY_KEY_*`, and the providers'
/// own names for theirs). Commands and MCP servers she starts don't inherit them.
pub fn secret_vars() -> Vec<String> {
    const VENDOR: &[&str] = &[
        "ANTHROPIC_API_KEY",
        "OPENAI_API_KEY",
        "OPENROUTER_API_KEY",
        "DASHSCOPE_API_KEY",
        "ELEVENLABS_API_KEY",
    ];
    std::env::vars_os()
        .filter_map(|(name, _)| name.into_string().ok())
        .filter(|name| name.starts_with("LUCY_KEY_") || VENDOR.contains(&name.as_str()))
        .collect()
}

/// Whether `url` names this machine or a private network itself (`localhost`, `127.0.0.1`,
/// `192.168.1.5`, `169.254.169.254`…), rather than a public name.
pub fn names_private_host(url: &str) -> bool {
    let Ok(url) = reqwest::Url::parse(url) else {
        return false;
    };
    match url.host() {
        Some(url::Host::Domain(name)) => {
            let name = name.trim_end_matches('.').to_ascii_lowercase();
            name == "localhost" || name.ends_with(".localhost")
        }
        Some(url::Host::Ipv4(ip)) => private_ip(ip.into()),
        Some(url::Host::Ipv6(ip)) => private_ip(ip.into()),
        None => false,
    }
}

/// This machine, a private or link-local network, or shared address space (Tailscale).
fn private_ip(ip: std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(ip) => {
            ip.is_loopback()
                || ip.is_private()
                || ip.is_link_local()
                || ip.is_unspecified()
                || ip.is_broadcast()
                || (ip.octets()[0] == 100 && (ip.octets()[1] & 0xc0) == 64)
        }
        std::net::IpAddr::V6(ip) => {
            ip.is_loopback()
                || ip.is_unspecified()
                || (ip.segments()[0] & 0xfe00) == 0xfc00
                || (ip.segments()[0] & 0xffc0) == 0xfe80
                || ip.to_ipv4_mapped().is_some_and(|v4| private_ip(v4.into()))
        }
    }
}

/// Resolves names to public addresses only, so a page can't send Lucy into the local network
/// by redirecting to (or being renamed as) a private address.
struct PublicOnly;

impl reqwest::dns::Resolve for PublicOnly {
    fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        let host = name.as_str().to_owned();
        Box::pin(async move {
            let public: Vec<std::net::SocketAddr> = tokio::net::lookup_host((host.as_str(), 0))
                .await?
                .filter(|addr| !private_ip(addr.ip()))
                .collect();
            if public.is_empty() {
                return Err(format!(
                    "{host} is on this machine or a private network; fetch it by that address \
                     to be asked about it"
                )
                .into());
            }
            Ok(Box::new(public.into_iter()) as reqwest::dns::Addrs)
        })
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for Builtin {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
    }
}

/// The file after an edit, or why the edit can't be made.
fn apply_edit(text: &str, args: &EditFileArgs, shown: &str) -> Result<String, String> {
    if args.old_text.is_empty() {
        return Err("old_text is empty; use write_file to create a file".into());
    }
    match text.matches(&args.old_text).count() {
        0 => Err(format!(
            "old_text not found in {shown}; read the file and copy the text exactly"
        )),
        1 => Ok(text.replacen(&args.old_text, &args.new_text, 1)),
        n if args.replace_all.unwrap_or(false) => {
            let _ = n;
            Ok(text.replace(&args.old_text, &args.new_text))
        }
        n => Err(format!(
            "old_text appears {n} times in {shown}; include more surrounding text, or set \
             replace_all to change every one"
        )),
    }
}

/// The folder a search walks: its real path, and the path it was asked for.
#[derive(Clone)]
struct Root {
    real: PathBuf,
    given: PathBuf,
}

impl Root {
    fn is_private(&self, path: &Path, private: &PrivatePaths) -> bool {
        private.contains(path)
            || path
                .strip_prefix(&self.real)
                .is_ok_and(|rel| private.contains(&self.given.join(rel)))
    }
}

fn search(
    root: &Root,
    cwd: &Path,
    regex: &regex::Regex,
    glob: Option<&globset::GlobMatcher>,
    private: &PrivatePaths,
) -> String {
    let walker = ignore::WalkBuilder::new(&root.real)
        .hidden(true)
        .require_git(false)
        .filter_entry({
            let private = private.clone();
            let root = root.clone();
            move |entry| !root.is_private(entry.path(), &private)
        })
        .build();
    let mut matches = Vec::new();
    let mut more = 0usize;
    for entry in walker.flatten() {
        if !entry.file_type().is_some_and(|t| t.is_file()) {
            continue;
        }
        let path = entry.path();
        let shown = display(path, cwd);
        if let Some(glob) = glob {
            let rel = path.strip_prefix(&root.real).unwrap_or(path);
            let name = path.file_name().map(Path::new).unwrap_or(rel);
            if !(glob.is_match(rel) || glob.is_match(name)) {
                continue;
            }
        }
        if entry.metadata().is_ok_and(|m| m.len() > MAX_SEARCH_FILE) {
            continue;
        }
        let Ok(bytes) = std::fs::read(path) else {
            continue;
        };
        if is_binary(&bytes) {
            continue;
        }
        for (n, line) in String::from_utf8_lossy(&bytes).lines().enumerate() {
            if regex.is_match(line) {
                if matches.len() < MAX_MATCHES {
                    let line: String = line.trim_end().chars().take(MAX_MATCH_LINE).collect();
                    matches.push(format!("{shown}:{}: {line}", n + 1));
                } else {
                    more += 1;
                }
            }
        }
    }
    if matches.is_empty() {
        return "No matches.".into();
    }
    if more > 0 {
        matches.push(format!(
            "[… and {more} more matches; narrow the pattern, path or glob]"
        ));
    }
    matches.join("\n")
}

async fn run(command: &str, cwd: &Path, timeout: Duration) -> Result<String, String> {
    let mut cmd = if cfg!(windows) {
        let mut cmd = tokio::process::Command::new("cmd");
        cmd.arg("/C").arg(command);
        cmd
    } else {
        let mut cmd = tokio::process::Command::new("sh");
        cmd.arg("-c").arg(command);
        cmd
    };
    for var in secret_vars() {
        cmd.env_remove(var);
    }
    cmd.current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    // Its own process group, so a timeout can stop everything the command started.
    #[cfg(unix)]
    cmd.process_group(0);
    let mut child = cmd
        .spawn()
        .map_err(|err| format!("could not start the shell: {err}"))?;

    let mut stdout = child.stdout.take().expect("stdout is piped");
    let mut stderr = child.stderr.take().expect("stderr is piped");
    let out = tokio::spawn(async move {
        let mut buf = Vec::new();
        let _ = stdout.read_to_end(&mut buf).await;
        buf
    });
    let err = tokio::spawn(async move {
        let mut buf = Vec::new();
        let _ = stderr.read_to_end(&mut buf).await;
        buf
    });

    let status = match tokio::time::timeout(timeout, child.wait()).await {
        Ok(status) => status.map_err(|err| format!("command failed: {err}"))?,
        Err(_) => {
            kill_tree(&mut child).await;
            out.abort();
            err.abort();
            return Err(format!(
                "timed out after {}s and was stopped",
                timeout.as_secs()
            ));
        }
    };
    // Background processes may hold the pipes open; don't wait on them for long.
    let grace = Duration::from_secs(2);
    let stdout = tokio::time::timeout(grace, out)
        .await
        .ok()
        .and_then(Result::ok)
        .unwrap_or_default();
    let stderr = tokio::time::timeout(grace, err)
        .await
        .ok()
        .and_then(Result::ok)
        .unwrap_or_default();

    let mut output = String::from_utf8_lossy(&stdout).into_owned();
    if !stderr.is_empty() {
        if !output.is_empty() && !output.ends_with('\n') {
            output.push('\n');
        }
        output.push_str("[stderr]\n");
        output.push_str(&String::from_utf8_lossy(&stderr));
    }
    let output = truncate_middle(output.trim_end(), MAX_OUTPUT);
    match status.code() {
        Some(0) if output.is_empty() => Ok("(no output)".into()),
        Some(0) => Ok(output),
        Some(code) => Err(format!("exit code {code}\n{output}")),
        None => Err(format!("killed by a signal\n{output}")),
    }
}

async fn kill_tree(child: &mut tokio::process::Child) {
    #[cfg(unix)]
    if let Some(pid) = child.id() {
        // SAFETY: plain syscall; a negative pid signals the whole process group.
        unsafe {
            libc::kill(-(pid as i32), libc::SIGKILL);
        }
    }
    let _ = child.kill().await;
}

/// A text file's contents, or the requested page of them.
fn read_text(path: &Path, bytes: &[u8], args: &ReadFileArgs) -> Result<String, String> {
    if is_binary(bytes) {
        return Err(format!("{} is a binary file", path.display()));
    }
    let text = String::from_utf8_lossy(bytes);
    let lines: Vec<&str> = text.split_inclusive('\n').collect();
    let start = args.offset.unwrap_or(1).max(1);
    let limit = args.limit.unwrap_or(READ_LINES).max(1);
    if start == 1 && lines.len() <= limit && text.len() <= MAX_OUTPUT {
        return Ok(text.into_owned());
    }
    if start > lines.len() {
        return Err(format!("{} has only {} lines", path.display(), lines.len()));
    }

    let mut page = String::new();
    let mut end = start - 1;
    for line in lines.iter().skip(start - 1).take(limit) {
        if page.len() + line.len() > MAX_OUTPUT && end >= start {
            break;
        }
        page.push_str(line);
        end += 1;
    }
    if end < lines.len() {
        if !page.ends_with('\n') {
            page.push('\n');
        }
        page.push_str(&format!(
            "[Showing lines {start}–{end} of {}. Read on with offset={}.]",
            lines.len(),
            end + 1
        ));
    }
    Ok(page)
}

/// NUL bytes near the start mean binary data.
fn is_binary(bytes: &[u8]) -> bool {
    bytes.iter().take(8192).any(|&b| b == 0)
}

fn size(bytes: u64) -> String {
    match bytes {
        b if b < 1024 => format!("{b} B"),
        b if b < 1024 * 1024 => format!("{:.1} KB", b as f64 / 1024.0),
        b => format!("{:.1} MB", b as f64 / (1024.0 * 1024.0)),
    }
}

fn truncate(text: &str, max: usize) -> String {
    if text.len() <= max {
        return text.to_owned();
    }
    let cut = floor_char_boundary(text, max);
    format!("{}\n[… cut off after {}]", &text[..cut], size(max as u64))
}

/// Keep the start and the end (where errors usually are) of a long output.
fn truncate_middle(text: &str, max: usize) -> String {
    if text.len() <= max {
        return text.to_owned();
    }
    let head = floor_char_boundary(text, max / 3);
    let mut tail = text.len() - (max - max / 3);
    while !text.is_char_boundary(tail) {
        tail += 1;
    }
    format!(
        "{}\n[… {} omitted …]\n{}",
        &text[..head],
        size((tail - head) as u64),
        &text[tail..]
    )
}

fn floor_char_boundary(text: &str, mut at: usize) -> usize {
    while !text.is_char_boundary(at) {
        at -= 1;
    }
    at
}

/// What a call would change, for the approval prompt: a diff for file writes and edits.
pub fn preview(tool: &str, arguments: &Value, cwd: &Path) -> Option<String> {
    const MAX_LINES: usize = 80;
    let path = resolve(arguments.get("path")?.as_str()?, cwd);
    let shown = display(&path, cwd);
    let old = std::fs::read_to_string(&path).unwrap_or_default();
    let new = match tool {
        "write_file" => arguments.get("content")?.as_str()?.to_owned(),
        "edit_file" => {
            let args: EditFileArgs = serde_json::from_value(arguments.clone()).ok()?;
            apply_edit(&old, &args, &shown).ok()?
        }
        _ => return None,
    };
    let diff = similar::TextDiff::from_lines(&old, &new);
    let text = diff
        .unified_diff()
        .context_radius(2)
        .header(&shown, &shown)
        .to_string();
    let lines: Vec<&str> = text.lines().collect();
    let mut preview = lines[..lines.len().min(MAX_LINES)].join("\n");
    if lines.len() > MAX_LINES {
        preview.push_str(&format!("\n[… {} more lines]", lines.len() - MAX_LINES));
    }
    Some(preview)
}

/// Serve the built-in tools over stdin/stdout, for other MCP clients (`lucy mcp serve`).
pub async fn serve_stdio(options: BuiltinOptions) -> Result<(), String> {
    let running = Builtin::new(options)
        .serve(rmcp::transport::stdio())
        .await
        .map_err(|err| format!("MCP handshake failed: {err}"))?;
    running
        .waiting()
        .await
        .map_err(|err| format!("MCP server stopped: {err}"))?;
    Ok(())
}

/// The type of a picture `read_file` can show, recognised by its first bytes.
fn picture_type(bytes: &[u8]) -> Option<&'static str> {
    match bytes {
        [0x89, b'P', b'N', b'G', ..] => Some("image/png"),
        [0xFF, 0xD8, 0xFF, ..] => Some("image/jpeg"),
        [b'G', b'I', b'F', b'8', ..] => Some("image/gif"),
        [
            b'R',
            b'I',
            b'F',
            b'F',
            _,
            _,
            _,
            _,
            b'W',
            b'E',
            b'B',
            b'P',
            ..,
        ] => Some("image/webp"),
        _ => None,
    }
}
