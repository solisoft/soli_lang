//! `GET /.well-known/eui` — the application manifest of the EUI
//! specification (01 §2.1), signed with the application's publisher key.
//!
//! The key is Ed25519, generated on first use and kept in
//! `config/eui_publisher.pkcs8` next to the application; a client pins the
//! public half on first connection and refuses a different one afterwards,
//! so losing this file means every existing user must re-pin. Commit it to
//! nothing public.

use std::sync::{Mutex, OnceLock};

use bytes::Bytes;
use eui_proto::Manifest;
use hyper::{header, Response, StatusCode};
use ring::rand::SystemRandom;
use ring::signature::{Ed25519KeyPair, KeyPair};

use crate::live::component::get_app_root;
use crate::serve::full;
use crate::serve::ResponseBody;

/// Capabilities the application requests, set by `eui_capabilities(...)`.
static CAPABILITIES: Mutex<u32> = Mutex::new(0);

/// `eui_capabilities("clipboard.read", ...)`: what the manifest asks for.
/// The client grants only what the person allows on top of this.
pub fn request_capabilities(names: &[String]) -> Result<(), String> {
    let mut mask = 0;
    for n in names {
        mask |= eui_proto::caps::from_name(n)
            .ok_or_else(|| format!("eui_capabilities: unknown capability '{n}'"))?;
    }
    *CAPABILITIES.lock().unwrap_or_else(|e| e.into_inner()) |= mask;
    Ok(())
}

fn key_pair() -> Result<&'static Ed25519KeyPair, String> {
    static KEY: OnceLock<Result<Ed25519KeyPair, String>> = OnceLock::new();
    KEY.get_or_init(|| {
        // A desktop artifact points this at its per-install state directory:
        // the developer's key must never travel inside a bundle.
        let path = std::env::var_os("SOLI_EUI_KEY")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| get_app_root().join("config").join("eui_publisher.pkcs8"));
        let pkcs8 = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let doc = Ed25519KeyPair::generate_pkcs8(&SystemRandom::new())
                    .map_err(|_| "EUI: cannot generate a publisher key".to_string())?;
                if let Some(dir) = path.parent() {
                    std::fs::create_dir_all(dir)
                        .map_err(|e| format!("EUI: {}: {e}", dir.display()))?;
                }
                std::fs::write(&path, doc.as_ref())
                    .map_err(|e| format!("EUI: {}: {e}", path.display()))?;
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
                }
                eprintln!(
                    "[EUI] generated the publisher key at {} — keep it, clients pin it",
                    path.display()
                );
                doc.as_ref().to_vec()
            }
            Err(e) => return Err(format!("EUI: {}: {e}", path.display())),
        };
        Ed25519KeyPair::from_pkcs8(&pkcs8)
            .map_err(|_| format!("EUI: {} is not an Ed25519 PKCS#8 key", path.display()))
    })
    .as_ref()
    .map_err(Clone::clone)
}

/// The manifest bytes, signed.
pub fn bytes() -> Result<Vec<u8>, String> {
    let key = key_pair()?;
    let root = get_app_root();
    let app_id = root
        .canonicalize()
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
        .unwrap_or_else(|| "app".into());
    let mut publisher_key = [0u8; 32];
    publisher_key.copy_from_slice(key.public_key().as_ref());
    let manifest = Manifest {
        app_id: app_id.clone(),
        name: app_id,
        version: env!("CARGO_PKG_VERSION").to_string(),
        protocol_min: 1,
        protocol_max: 1,
        publisher_key,
        capabilities: *CAPABILITIES.lock().unwrap_or_else(|e| e.into_inner()),
        theme: None,
        entry: "/_eui/session".into(),
        rotation: None,
    };
    let signature = key.sign(&manifest.signed_bytes());
    let mut sig = [0u8; 64];
    sig.copy_from_slice(signature.as_ref());
    Ok(manifest.encode(&sig))
}

/// The HTTP answer.
pub fn respond() -> Response<ResponseBody> {
    match bytes() {
        Ok(body) => Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "application/vnd.eui.manifest")
            .header(header::CACHE_CONTROL, "no-cache")
            .header(header::CONTENT_LENGTH, body.len())
            .body(full(Bytes::from(body)))
            .unwrap_or_else(|_| Response::new(full(Bytes::new()))),
        Err(e) => {
            eprintln!("[EUI] manifest: {e}");
            Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .body(full(Bytes::from("manifest unavailable")))
                .unwrap_or_else(|_| Response::new(full(Bytes::new())))
        }
    }
}
