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

/// What this application is called, set by `eui_name("...")`.
///
/// The manifest has always had a `name`; what it never had was a way to
/// say one, so it was the directory's name and an installed application
/// wore that on its dock tile — `demo-app.app`. A directory name is a
/// path, not a title, and the two only look alike while the application
/// happens to live somewhere tidy.
static NAME: TenantValue<Option<String>> = TenantValue::new(|| None);

/// `eui_name("Demo")`: what a client shows for this application — the tab
/// strip, the launcher entry, the dock tile.
pub fn declare_name(name: &str) -> Result<(), String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("eui_name: a name with nothing in it is not a name".into());
    }
    // 01 §2.1 caps every manifest string, and a name past it would be
    // refused by the client after the signature verified — which reads as
    // a broken manifest rather than as a name somebody made too long.
    if name.len() > 256 {
        return Err(format!(
            "eui_name: at most 256 bytes, this is {}",
            name.len()
        ));
    }
    NAME.write(|held| *held = Some(name.to_owned()));
    Ok(())
}

/// The icon this application is installed as, set by `eui_icon("...")`.
///
/// Per application for the same reason the capabilities are: the picture
/// on a co-hosted application's launcher tile must be its own.
static ICON: TenantValue<Option<String>> = TenantValue::new(|| None);

/// Where an application's icon is looked for when it did not say. A file
/// with this name and nothing else to declare is the whole of publishing
/// one, which is what `favicon.ico` got right.
const ICON_BY_CONVENTION: [&str; 2] = ["public/icon.png", "public/images/icon.png"];

/// `eui_icon("public/images/logo.png")`: the PNG a client installs this
/// application as (EUI 01 §2.1).
///
/// A path and not bytes, resolved at signing time through the same asset
/// store a view's pictures go through — so it must live under `public/` or
/// `app/assets/`, and the client fetches it by its hash like anything else.
pub fn declare_icon(path: &str) -> Result<(), String> {
    // Read now, so that a path that is not there is an error at boot with
    // the line number on it, rather than a manifest that quietly has no
    // icon in it and an Install button that never appears.
    super::assets::from_file(path)?;
    ICON.write(|icon| *icon = Some(path.to_owned()));
    Ok(())
}

/// The icon's hash, or `None` when this application has none to publish.
///
/// A declared path that has stopped resolving is not an error here: the
/// manifest is what a session depends on, and refusing to serve one
/// because a picture was moved would take the application down over its
/// launcher tile. It is said once on stderr and the manifest goes out
/// without an icon, which costs exactly the ability to install it.
fn icon_hash() -> Option<[u8; 32]> {
    if let Some(declared) = ICON.read(Clone::clone) {
        return match super::assets::from_file(&declared) {
            Ok(h) => Some(h),
            Err(e) => {
                eprintln!("EUI: the icon this application declared is not there ({e}); it will not be installable");
                None
            }
        };
    }
    ICON_BY_CONVENTION
        .iter()
        .find_map(|rel| super::assets::from_file(rel).ok())
}

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
    /// What a signed manifest was signed *for*: the capability mask, and
    /// whether the application had declared a font — the second raises
    /// `protocol_min`, so a manifest signed before `eui_font` ran is the
    /// wrong one to keep handing out.
    type SignedFor = (u32, bool, Option<[u8; 32]>, Option<String>);
    /// The signed manifest, cached against that.
    static SIGNED: TenantValue<Option<(SignedFor, Vec<u8>)>> = TenantValue::new(|| None);
    let capabilities = CAPABILITIES.read(|caps| *caps);
    // The icon is part of what the signature covers, so a picture that
    // changed on disk has to re-sign: its hash is in the cache key.
    let icon = icon_hash();
    let name = NAME.read(Clone::clone);
    let signed_for = (capabilities, super::fonts::any(), icon, name.clone());
    if let Some((held, bytes)) = SIGNED.read(|signed| signed.clone()) {
        if held == signed_for {
            return Ok(bytes);
        }
    }
    let bytes = sign(capabilities, icon, name)?;
    SIGNED.write(|signed| *signed = Some((signed_for, bytes.clone())));
    Ok(bytes)
}

fn sign(
    capabilities: u32,
    icon: Option<[u8; 32]>,
    name: Option<String>,
) -> Result<Vec<u8>, String> {
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
        // The directory's name identifies the application and is a poor
        // title for it; `eui_name` is how an application says the second
        // without changing the first, which is what a key is pinned to.
        name: name.unwrap_or_else(|| app_id.clone()),
        app_id,
        version: env!("CARGO_PKG_VERSION").to_string(),
        // The range this server serves. `scene` is the one thing in it that
        // an older client cannot be shown at all -- `0x11` is a decode error
        // there, and the batch carrying it would end the session -- so an
        // application that asked for it raises the floor and is refused at
        // the handshake instead, with a reason, before anything is drawn.
        // Every other application keeps its old clients.
        //
        // An *event* added in a later version cannot raise this floor the
        // same way, and should not try: a capability is declared before
        // anything renders, while a handler is a key in a view that has
        // not run yet. `level` (03 §7) is handled a step later instead --
        // `tree::since` leaves the handler out of the tree when the
        // session settled below the version it needs, so an old client
        // gets an application that works and a meter that does not move,
        // rather than a decode error and no session at all.
        // A declared font raises the floor for the same reason a `scene`
        // does, and higher: `DefFont` is opcode `0x15` and a font role is a
        // style byte of `2`, both decode errors before EUI 4. An
        // application that declared one is refused at the handshake, with a
        // reason, rather than mid-batch with a dead session.
        protocol_min: if super::fonts::any() {
            4
        } else if capabilities & eui_proto::caps::SCENE != 0 {
            2
        } else {
            1
        },
        protocol_max: eui_proto::PROTOCOL_VERSION,
        publisher_key,
        capabilities,
        theme: None,
        icon,
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
