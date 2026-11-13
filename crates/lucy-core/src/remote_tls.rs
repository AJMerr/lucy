//! The core's own TLS certificate for remote access: self-signed, made on first use and kept
//! in the keyring. Clients don't check it against a certificate authority; they pin its
//! fingerprint, which `lucy remote token` prints and `lucy remote connect` asks for.

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::secrets::{SecretStore, Secrets};

/// The keyring entry holding the certificate and its private key (JSON).
pub const TLS_KEY: &str = "remote-tls";

/// A certificate and its private key, both DER.
#[derive(Clone)]
pub struct Identity {
    pub cert: Vec<u8>,
    pub key: Vec<u8>,
}

#[derive(Serialize, Deserialize)]
struct Stored {
    cert: String,
    key: String,
}

impl Identity {
    /// The certificate's SHA-256, as clients pin it.
    pub fn fingerprint(&self) -> String {
        fingerprint(&self.cert)
    }
}

/// The SHA-256 of a DER certificate, as lowercase hex in groups of four for reading aloud.
pub fn fingerprint(cert: &[u8]) -> String {
    let hex: Vec<String> = Sha256::digest(cert)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    hex.chunks(2)
        .map(|pair| pair.concat())
        .collect::<Vec<_>>()
        .join(" ")
}

/// The certificate in the keyring, made (and kept) the first time.
pub fn load_or_create<S, E>(secrets: &Secrets<S, E>) -> Result<Identity, String>
where
    S: SecretStore,
    E: Fn(&str) -> Option<String>,
{
    let stored = secrets
        .resolve(TLS_KEY)
        .map_err(|err| format!("couldn't read the remote access certificate: {err}"))?;
    if let Some(stored) = stored
        && let Ok(stored) = serde_json::from_str::<Stored>(&stored.value)
        && let (Ok(cert), Ok(key)) = (STANDARD.decode(stored.cert), STANDARD.decode(stored.key))
    {
        return Ok(Identity { cert, key });
    }
    let made = rcgen::generate_simple_self_signed(vec!["lucy-core".to_owned()])
        .map_err(|err| format!("couldn't make a certificate for remote access: {err}"))?;
    let identity = Identity {
        cert: made.cert.der().to_vec(),
        key: made.signing_key.serialize_der(),
    };
    let stored = serde_json::to_string(&Stored {
        cert: STANDARD.encode(&identity.cert),
        key: STANDARD.encode(&identity.key),
    })
    .map_err(|err| err.to_string())?;
    secrets
        .set(TLS_KEY, &stored)
        .map_err(|err| format!("couldn't keep the remote access certificate: {err}"))?;
    Ok(identity)
}
