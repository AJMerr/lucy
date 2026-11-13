fn main() {
    // Every command is declared, so each window can be given only the ones it uses
    // (capabilities/*.json): the sprite, which shows her replies, can't reach the core directly.
    let commands = &[
        "pack",
        "send",
        "approve",
        "voice_prepare",
        "voice_status",
        "voice_listen",
        "voice_stop_listening",
        "voice_say",
        "voice_hush",
        "picture",
        "model",
        "open_chat",
        "take_pending_chat",
        "rpc_call",
        "run_lucy",
        "set_layout",
    ];
    tauri_build::try_build(
        tauri_build::Attributes::new()
            .app_manifest(tauri_build::AppManifest::new().commands(commands)),
    )
    .expect("failed to run the Tauri build script");
}
