//! `GET /.well-known/eui` — the application manifest of the EUI
//! specification (01 §2.1), signed with the application's publisher key.
//!
//! The key is Ed25519, generated on first use and kept in
//! `config/eui_publisher.pkcs8` next to the application; a client pins the
//! public half on first connection and refuses a different one afterwards,
//! so losing this file means every existing user must re-pin. Commit it to
//! nothing public.

use bytes::Bytes;
use eui_proto::Manifest;
use hyper::{header, Response, StatusCode};
use ring::rand::SystemRandom;
use ring::signature::{Ed25519KeyPair, KeyPair};

use crate::serve::full;
use crate::serve::tenant::{app_root as get_app_root, TenantValue};
use crate::serve::ResponseBody;

/// Capabilities the application requests, set by `eui_capabilities(...)`.
///
/// Per application: this is what one app asks the client's user to grant it.
/// Shared, a co-hosted app would inherit permissions it never requested.
static CAPABILITIES: TenantValue<u32> = TenantValue::new(|| 0);

/// `eui_capabilities("clipboard.read", ...)`: what the manifest asks for.
/// The client grants only what the person allows on top of this.
pub fn request_capabilities(names: &[String]) -> Result<(), String> {
    let mut mask = 0;
    for n in names {
        mask |= eui_proto::caps::from_name(n)
            .ok_or_else(|| format!("eui_capabilities: unknown capability '{n}'"))?;
    }
    CAPABILITIES.write(|caps| *caps |= mask);
    Ok(())
}

/// This application's Ed25519 publisher key, loaded (or generated) on first use.
///
/// Per application, and this one is a *private key*: it is read from that
/// application's `config/eui_publisher.pkcs8`, and clients pin the public half.
/// A process-wide key would have signed every co-hosted application's manifest
/// with whichever one booted first — so a client that pinned one publisher
/// would silently accept manifests from another.
///
/// Held as an `Arc` because `Ed25519KeyPair` is neither `Clone` nor lendable
/// out of the lock it lives in.
static KEY: TenantValue<std::sync::Arc<Result<Ed25519KeyPair, String>>> =
    TenantValue::new(load_key_pair);

fn key_pair() -> Result<std::sync::Arc<Result<Ed25519KeyPair, String>>, String> {
    let key = KEY.read(std::sync::Arc::clone);
    match key.as_ref() {
        Ok(_) => Ok(key),
        Err(e) => Err(e.clone()),
    }
}

fn load_key_pair() -> std::sync::Arc<Result<Ed25519KeyPair, String>> {
    std::sync::Arc::new(read_or_generate_key())
}

fn read_or_generate_key() -> Result<Ed25519KeyPair, String> {
    {
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
                // Created private: a write-then-chmod left the key readable
                // by everyone for the moment between the two.
                let mut options = std::fs::OpenOptions::new();
                options.write(true).create_new(true);
                #[cfg(unix)]
                {
                    use std::os::unix::fs::OpenOptionsExt;
                    options.mode(0o600);
                }
                {
                    use std::io::Write;
                    let mut file = options
                        .open(&path)
                        .map_err(|e| format!("EUI: {}: {e}", path.display()))?;
                    file.write_all(doc.as_ref())
                        .map_err(|e| format!("EUI: {}: {e}", path.display()))?;
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
    }
}

/// The manifest bytes, signed — once per capability mask. Every request
/// used to resolve the root and sign again.
pub fn bytes() -> Result<Vec<u8>, String> {
    /// The signed manifest, cached against the capability mask it was signed
    /// for. Per application, like the mask itself.
    static SIGNED: TenantValue<Option<(u32, Vec<u8>)>> = TenantValue::new(|| None);
    let capabilities = CAPABILITIES.read(|caps| *caps);
    if let Some((mask, bytes)) = SIGNED.read(|signed| signed.clone()) {
        if mask == capabilities {
            return Ok(bytes);
        }
    }
    let bytes = sign(capabilities)?;
    SIGNED.write(|signed| *signed = Some((capabilities, bytes.clone())));
    Ok(bytes)
}

fn sign(capabilities: u32) -> Result<Vec<u8>, String> {
    let key = key_pair()?;
    let key = key.as_ref().as_ref().map_err(Clone::clone)?;
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
        capabilities,
        theme: None,
        // Where this application's session lives, so a client need not be
        // told the protocol's own prefix: `wss://host` is an address, and
        // this is what completes it (EUI 01 §2.1). The first `router_eui`
        // in the routes file, or whichever one said `{"default": true}`.
        entry: super::entry_path(),
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
