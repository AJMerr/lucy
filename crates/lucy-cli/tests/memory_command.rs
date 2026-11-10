//! `lucy memory`: shows what she remembers, and `lucy memory edit` changes it in an editor.

use std::time::Duration;

use lucy_cli::memory_command::{edit, show};
use lucy_client::{Client, Endpoint};
use lucy_core::secrets::{MemoryStore, Secrets};
use lucy_core::server::{CoreOptions, serve};

async fn core(memory: &str) -> (tempfile::TempDir, Client) {
    let dir = tempfile::tempdir().unwrap();
    let config_path = dir.path().join("config.toml");
    std::fs::write(
        &config_path,
        "[defaults]\nprofile = \"fake\"\n\n[profiles.fake]\nprovider = \"openai_compat\"\n\
         base_url = \"http://127.0.0.1:9/v1\"\nmodel = \"m\"\n",
    )
    .unwrap();
    std::fs::write(dir.path().join("memory.md"), memory).unwrap();
    let endpoint = if cfg!(windows) {
        let unique = dir.path().file_name().unwrap().to_string_lossy();
        Endpoint::Pipe(format!("lucy-memory-cmd-{unique}"))
    } else {
        Endpoint::Path(dir.path().join("core.sock"))
    };
    let options = CoreOptions {
        endpoint: endpoint.clone(),
        config_path,
        db_path: dir.path().join("lucy.sqlite"),
        personas_dir: None,
        secrets: Secrets::new(Box::new(MemoryStore::default()), Box::new(|_: &str| None)),
        idle_timeout: Some(Duration::from_secs(3600)),
        token_store: None,
    };
    tokio::spawn(async move { serve(options).await.unwrap() });
    for _ in 0..200 {
        if let Ok(client) = Client::connect(&endpoint, "test").await {
            return (dir, client);
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("the core didn't start");
}

#[tokio::test]
async fn lucy_memory_prints_what_she_remembers() {
    let (_dir, client) = core("## About you\n- Has a dog called Biscuit\n").await;
    let mut out = Vec::new();

    show(&client, &mut out).await.unwrap();

    let out = String::from_utf8(out).unwrap();
    assert!(out.contains("Has a dog called Biscuit"), "{out}");
    assert!(out.contains("memory.md"), "says where it lives: {out}");
}

#[tokio::test]
async fn an_empty_memory_says_so() {
    let (_dir, client) = core("").await;
    let mut out = Vec::new();

    show(&client, &mut out).await.unwrap();

    assert!(String::from_utf8(out).unwrap().contains("nothing yet"));
}

#[tokio::test]
async fn lucy_memory_edit_saves_what_you_change_in_the_editor() {
    let (dir, client) = core("## About you\n- Has a dog called Biscuit\n").await;

    edit(&client, |file| {
        let text = std::fs::read_to_string(file).unwrap();
        std::fs::write(file, text.replace("Biscuit", "Pepper")).unwrap();
        Ok(())
    })
    .await
    .unwrap();

    let saved = std::fs::read_to_string(dir.path().join("memory.md")).unwrap();
    assert_eq!(saved, "## About you\n- Has a dog called Pepper\n");
}
