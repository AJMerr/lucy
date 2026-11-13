//! A local socket is only trusted in a folder of the user's own that nobody else can write to.

#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;

use lucy_client::{Client, ClientError, Endpoint};

#[tokio::test]
async fn a_socket_in_a_folder_others_can_write_to_isnt_trusted() {
    let dir = tempfile::tempdir().unwrap();
    let shared = dir.path().join("shared");
    std::fs::create_dir(&shared).unwrap();
    std::fs::set_permissions(&shared, std::fs::Permissions::from_mode(0o777)).unwrap();
    let _listener = std::os::unix::net::UnixListener::bind(shared.join("core.sock")).unwrap();

    let refused = Client::connect(&Endpoint::Path(shared.join("core.sock")), "test")
        .await
        .err()
        .expect("connected through a folder anyone can write to");

    assert!(
        matches!(&refused, ClientError::Connect { source, .. } if source.to_string().contains("others can write")),
        "{refused}"
    );
}
