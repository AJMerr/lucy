//! `run_lucy`: the settings page's way to run a few `lucy` commands on this machine (the login
//! service and the remote-access token), which aren't in the core's protocol because they
//! manage the core itself. Only the commands listed here run.

use std::process::Command;

/// The commands the page may run, as their arguments to `lucy`.
const ALLOWED: &[&[&str]] = &[
    &["service", "status"],
    &["service", "install"],
    &["service", "uninstall"],
    &["service", "restart"],
    &["remote", "token"],
    &["remote", "token", "--rotate"],
];

/// Run `lucy <args>`; answers with what it printed, or that as an error if it failed.
#[tauri::command]
pub async fn run_lucy(args: Vec<String>) -> Result<String, String> {
    if !ALLOWED.iter().any(|allowed| allowed.iter().eq(args.iter())) {
        return Err(format!("`lucy {}` can't be run from here", args.join(" ")));
    }
    let exe = lucy_cli::core_process::lucy_exe().map_err(|err| err.to_string())?;
    let output = tauri::async_runtime::spawn_blocking(move || {
        Command::new(exe)
            .args(&args)
            .stdin(std::process::Stdio::null())
            .output()
    })
    .await
    .map_err(|err| err.to_string())?
    .map_err(|err| format!("couldn't run lucy: {err}"))?;
    let mut text = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    let errors = String::from_utf8_lossy(&output.stderr);
    if !errors.trim().is_empty() {
        if !text.is_empty() {
            text.push('\n');
        }
        text += errors.trim();
    }
    if output.status.success() {
        Ok(text)
    } else {
        Err(text)
    }
}
