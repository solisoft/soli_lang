use std::collections::HashMap;
use std::path::{Component, Path};
use std::sync::OnceLock;

use crate::ast::Program;

const BUNDLE_MAGIC: &[u8; 4] = b"SOLB";

/// Magic of an encrypted bundle container: `SOLE` + version byte + nonce[12]
/// + AES-256-GCM ciphertext (incl. 16-byte tag) of the plaintext SOLB bundle.
pub const ENCRYPTED_BUNDLE_MAGIC: &[u8; 4] = b"SOLE";
const ENCRYPTED_BUNDLE_VERSION: u8 = 1;
/// magic(4) + version(1) + nonce(12) + GCM tag(16)
const ENCRYPTED_BUNDLE_MIN_LEN: usize = 4 + 1 + 12 + 16;

/// Message shown by the plain-bundle parsers when handed an encrypted bundle.
pub const ENCRYPTED_BUNDLE_HINT: &str = "this bundle is encrypted — provide the decryption \
     key via SOLI_BUNDLE_KEY, or SOLI_BUNDLE_AUTH_URL (+ SOLI_BUNDLE_API_KEY), in the \
     environment or in a .env file next to the .soli bundle";

/// Reject a bundle entry path that could escape the extraction root: absolute
/// paths, `..` traversal, or Windows prefixes/roots (Zip-Slip). Bundle entries
/// are always relative in-app paths, so anything else is malicious — a crafted
/// `.soli` could otherwise make `serve_from_bundle` write outside the temp dir
/// (e.g. `../../.ssh/authorized_keys`) during extraction, before any code runs.
/// Mirrors the hardened tar extractor (SEC-075).
pub fn validate_entry_path(path: &str) -> Result<(), String> {
    let mut saw_component = false;
    for component in Path::new(path).components() {
        match component {
            Component::Normal(_) | Component::CurDir => saw_component = true,
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(format!("Invalid bundle: unsafe entry path '{}'", path));
            }
        }
    }
    if !saw_component {
        return Err("Invalid bundle: empty entry path".to_string());
    }
    Ok(())
}

/// True when the bytes look like an encrypted (`SOLE`) bundle container.
pub fn is_encrypted_bundle(data: &[u8]) -> bool {
    data.len() >= 4 && &data[..4] == ENCRYPTED_BUNDLE_MAGIC
}

/// Wrap plaintext SOLB bundle bytes in the encrypted container. The AES-256
/// key is SHA-256 of the (opaque, UTF-8) key material — hex, base64 or a
/// passphrase all work, as long as build and serve use the same string.
pub fn encrypt_bundle(plain: &[u8], key_material: &str) -> Result<Vec<u8>, String> {
    use crate::interpreter::builtins::crypto::{aes_encrypt_bytes, derive_aes_key};

    if plain.len() < 4 || &plain[..4] != BUNDLE_MAGIC {
        return Err("encrypt_bundle: input is not a SOLB bundle".to_string());
    }
    let key = derive_aes_key(key_material.as_bytes());
    let sealed = aes_encrypt_bytes(plain, &key)?; // nonce[12] ‖ ct+tag
    let mut out = Vec::with_capacity(5 + sealed.len());
    out.extend_from_slice(ENCRYPTED_BUNDLE_MAGIC);
    out.push(ENCRYPTED_BUNDLE_VERSION);
    out.extend_from_slice(&sealed);
    Ok(out)
}

/// Decrypt an encrypted (`SOLE`) container back to plaintext SOLB bytes.
pub fn decrypt_bundle(data: &[u8], key_material: &str) -> Result<Vec<u8>, String> {
    use crate::interpreter::builtins::crypto::{aes_decrypt_bytes, derive_aes_key};

    if !is_encrypted_bundle(data) {
        return Err("decrypt_bundle: not an encrypted bundle (bad magic)".to_string());
    }
    if data.len() < ENCRYPTED_BUNDLE_MIN_LEN {
        return Err("invalid encrypted bundle: truncated".to_string());
    }
    let version = data[4];
    if version != ENCRYPTED_BUNDLE_VERSION {
        return Err(format!(
            "unsupported encrypted bundle version {version} — upgrade soli"
        ));
    }
    let key = derive_aes_key(key_material.as_bytes());
    let plain = aes_decrypt_bytes(&data[5..], &key).map_err(|_| {
        "bundle decryption failed: wrong or rotated key, or corrupted file".to_string()
    })?;
    if plain.len() < 4 || &plain[..4] != BUNDLE_MAGIC {
        return Err(
            "bundle decrypted but the payload is not a SOLB bundle — corrupted build?".to_string(),
        );
    }
    Ok(plain)
}

/// What gets bundled. Code and templates are the obvious half; the rest is
/// everything a served page asks for afterwards.
///
/// Static assets are not optional. A bundle that carries `.css` and `.js` but
/// no images, icons, fonts or `.html` produces an app that boots and then
/// renders without its logo, its favicon, its web-app manifest and its offline
/// page — all 404, all silently. Anything a browser can request off `public/`
/// belongs here.
///
/// Entries are stored and served as bytes (`VirtualFileSystem::read`), so
/// binary formats are safe; only `read_to_string` assumes UTF-8, and nothing
/// calls it on an asset.
const BUNDLE_EXTENSIONS: &[&str] = &[
    // Code, templates, configuration
    "sl",
    "slv",
    "yml",
    "yaml",
    "erb",
    "toml",
    "json",
    "env",
    "md", // Documents and styles
    "css",
    "js",
    "mjs",
    "map",
    "html",
    "htm",
    "txt",
    "xml",
    "webmanifest",
    // Images
    "png",
    "jpg",
    "jpeg",
    "gif",
    "svg",
    "webp",
    "avif",
    "ico",
    "bmp",
    // Fonts
    "woff",
    "woff2",
    "ttf",
    "otf",
    "eot", // Media
    "mp3",
    "wav",
    "ogg",
    "oga",
    "m4a",
    "mp4",
    "webm",
    "vtt",
];

const BUNDLE_SPECIAL_FILES: &[&str] = &["soli.toml", ".solivrc"];

pub struct BundleBuilder;

impl BundleBuilder {
    pub fn build(source_dir: &Path) -> Result<Vec<u8>, String> {
        let entries = Self::collect(source_dir)?;
        Self::serialize(&entries)
    }

    /// Build a PROTECTED bundle: every `.sl` source is replaced by its
    /// serialized binary AST (`SLAST` blob) so no readable source ships, and
    /// the metadata the serve pipeline normally scrapes from source text
    /// (middleware directives, controller registry info) is precomputed into
    /// a `__soli_meta__` entry.
    pub fn build_protected(source_dir: &Path) -> Result<Vec<u8>, String> {
        let mut entries = Self::collect(source_dir)?;

        if entries
            .keys()
            .any(|p| p.starts_with("engines/") && p.ends_with(".sl"))
        {
            return Err(
                "--protect does not yet support apps with engines/ (their controller \
                 metadata cannot be precomputed). Build without --protect, or move the \
                 engine code into the app."
                    .to_string(),
            );
        }

        let mut meta = BundleMeta {
            soli_version: env!("CARGO_PKG_VERSION").to_string(),
            ast_format: AST_FORMAT_VERSION,
            protected: true,
            middleware: HashMap::new(),
            controllers: Vec::new(),
            controller_superclasses: HashMap::new(),
        };

        let sl_paths: Vec<String> = entries
            .keys()
            .filter(|p| p.ends_with(".sl"))
            .cloned()
            .collect();

        for path in sl_paths {
            let content = entries.get(&path).expect("key from entries").clone();
            let source =
                String::from_utf8(content).map_err(|_| format!("'{}' is not valid UTF-8", path))?;

            let program = parse_source(&source).map_err(|e| format!("{}: {}", path, e))?;

            // Middleware order/global_only/scope_only directives live in
            // comments, which the AST does not keep — precompute them.
            if path.starts_with("app/middleware/") {
                let directives = crate::serve::middleware::extract_middleware_functions(&source);
                if !directives.is_empty() {
                    meta.middleware.insert(path.clone(), directives);
                }
            }

            // The controller registry (actions, before/after hooks, layouts)
            // is scraped from source text at boot — precompute it.
            if let Some(rel) = path.strip_prefix("app/controllers/") {
                let stem = rel.trim_end_matches(".sl");
                let file_name = stem.rsplit('/').next().unwrap_or(stem);
                if file_name.ends_with("_controller") {
                    let route_key = {
                        let mut segments: Vec<&str> = stem.split('/').collect();
                        if let Some(last) = segments.last_mut() {
                            *last = last.strip_suffix("_controller").unwrap_or(last);
                        }
                        segments.join("/")
                    };
                    let info =
                        crate::interpreter::builtins::controller::registry::parse_controller_source(
                            &source, file_name, &route_key,
                        )
                        .map_err(|e| format!("{}: {}", path, e))?;
                    if let Some(parent) =
                        crate::interpreter::builtins::controller::registry::extract_superclass_name(
                            &source,
                        )
                    {
                        if parent != "Controller" {
                            meta.controller_superclasses
                                .insert(info.name.clone(), parent);
                        }
                    }
                    meta.controllers.push(info);
                }
            }

            entries.insert(path, serialize_program(&program)?);
        }

        let meta_json = serde_json::to_vec(&meta)
            .map_err(|e| format!("failed to serialize bundle meta: {}", e))?;
        entries.insert(BUNDLE_META_ENTRY.to_string(), meta_json);

        Self::serialize(&entries)
    }

    fn collect(source_dir: &Path) -> Result<HashMap<String, Vec<u8>>, String> {
        let mut entries: HashMap<String, Vec<u8>> = HashMap::new();
        let source_dir = source_dir.canonicalize().map_err(|e| {
            format!(
                "Failed to resolve source directory '{}': {}",
                source_dir.display(),
                e
            )
        })?;

        Self::collect_entries(&source_dir, &source_dir, &mut entries)?;

        // Also look for special files at root
        for special in BUNDLE_SPECIAL_FILES {
            let special_path = source_dir.join(special);
            if special_path.is_file() {
                let data = std::fs::read(&special_path)
                    .map_err(|e| format!("Failed to read '{}': {}", special_path.display(), e))?;
                entries.insert(special.to_string(), data);
            }
        }

        Ok(entries)
    }

    fn collect_entries(
        source_dir: &Path,
        current_dir: &Path,
        entries: &mut HashMap<String, Vec<u8>>,
    ) -> Result<(), String> {
        let read_dir = std::fs::read_dir(current_dir).map_err(|e| {
            format!(
                "Failed to read directory '{}': {}",
                current_dir.display(),
                e
            )
        })?;

        for entry in read_dir {
            let entry = entry.map_err(|e| format!("Failed to read entry: {}", e))?;
            let path = entry.path();
            let file_name = entry.file_name();
            let name_str = file_name.to_string_lossy();

            // Skip hidden files and common non-app directories
            if name_str.starts_with('.') || name_str == "node_modules" || name_str == "target" {
                continue;
            }

            if path.is_dir() {
                Self::collect_entries(source_dir, &path, entries)?;
            } else if path.is_file() {
                // Check if the extension is one we want
                let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
                let is_special = BUNDLE_SPECIAL_FILES.iter().any(|s| *s == name_str.as_ref());
                let is_bundled_ext = BUNDLE_EXTENSIONS.contains(&ext);

                if is_special || is_bundled_ext {
                    let relative = path
                        .strip_prefix(source_dir)
                        .map_err(|e| format!("Failed to compute relative path: {}", e))?;
                    // Bundle entry keys are `/`-separated on every platform:
                    // every consumer (BundleFS lookups, the `app/controllers/`
                    // and `engines/` prefix checks in `build_protected`) matches
                    // on `/`. Using the native separator made Windows-built
                    // bundles silently ship an empty controller registry.
                    let relative_str = crate::virtual_fs::to_vfs_key(relative);

                    let data = std::fs::read(&path)
                        .map_err(|e| format!("Failed to read '{}': {}", path.display(), e))?;
                    entries.insert(relative_str, data);
                }
            }
        }

        Ok(())
    }

    /// Serialize an arbitrary entry set to `SOLB` bytes.
    ///
    /// Exposed for callers that assemble entries programmatically rather than
    /// by walking a source tree — the desktop container embeds a database
    /// binary and reference data, neither of which is a project file.
    pub fn serialize_entries(entries: &HashMap<String, Vec<u8>>) -> Result<Vec<u8>, String> {
        Self::serialize(entries)
    }

    fn serialize(entries: &HashMap<String, Vec<u8>>) -> Result<Vec<u8>, String> {
        let mut buf = Vec::new();

        // Magic
        buf.extend_from_slice(BUNDLE_MAGIC);

        // Entry count
        let count = entries.len() as u32;
        buf.extend_from_slice(&count.to_le_bytes());

        // Sort entries by path for deterministic output
        let mut sorted: Vec<(&String, &Vec<u8>)> = entries.iter().collect();
        sorted.sort_by(|a, b| a.0.cmp(b.0));

        for (path, content) in sorted {
            let path_bytes = path.as_bytes();
            let path_len = path_bytes.len() as u32;
            buf.extend_from_slice(&path_len.to_le_bytes());
            buf.extend_from_slice(path_bytes);

            let content_len = content.len() as u64;
            buf.extend_from_slice(&content_len.to_le_bytes());
            buf.extend_from_slice(content);
        }

        Ok(buf)
    }
}

// ---------------------------------------------------------------------------
// Binary AST blobs (`--protect`): a `.sl` entry whose content is
// `SLAST` + format-version byte + MessagePack of the parsed `Program`.
// ---------------------------------------------------------------------------

pub const AST_BLOB_MAGIC: &[u8; 5] = b"SLAST";
/// Bump on ANY change to the AST types: rmp of derived enums is not stable
/// across variant/field reordering, so a mismatch must be a hard error.
pub const AST_FORMAT_VERSION: u8 = 1;

/// True when the bytes are a serialized-AST blob rather than source text.
pub fn is_ast_blob(data: &[u8]) -> bool {
    data.len() > 6 && &data[..5] == AST_BLOB_MAGIC
}

/// Lex + parse source text into a `Program` (build-time helper).
pub fn parse_source(source: &str) -> Result<Program, String> {
    let tokens = crate::lexer::Scanner::new(source)
        .scan_tokens()
        .map_err(|e| format!("lex error: {}", e))?;
    crate::parser::Parser::new(tokens)
        .parse()
        .map_err(|e| format!("parse error: {}", e))
}

/// Serialize a parsed program into an `SLAST` blob.
pub fn serialize_program(program: &Program) -> Result<Vec<u8>, String> {
    let body =
        rmp_serde::to_vec(program).map_err(|e| format!("AST serialization failed: {}", e))?;
    let mut out = Vec::with_capacity(6 + body.len());
    out.extend_from_slice(AST_BLOB_MAGIC);
    out.push(AST_FORMAT_VERSION);
    out.extend_from_slice(&body);
    Ok(out)
}

/// Deserialize an `SLAST` blob back into a `Program`.
pub fn deserialize_program(data: &[u8]) -> Result<Program, String> {
    if !is_ast_blob(data) {
        return Err("not a serialized-AST blob".to_string());
    }
    let version = data[5];
    if version != AST_FORMAT_VERSION {
        return Err(format!(
            "AST blob format v{} does not match this soli (v{}) — rebuild the bundle with the \
             soli version that serves it",
            version, AST_FORMAT_VERSION
        ));
    }
    rmp_serde::from_slice(&data[6..])
        .map_err(|e| format!("AST deserialization failed (rebuild the bundle): {}", e))
}

// ---------------------------------------------------------------------------
// Bundle metadata (`__soli_meta__`): carries the soli version lock plus the
// serve-pipeline facts that are normally scraped from source text and are
// unavailable once sources are shipped as ASTs.
// ---------------------------------------------------------------------------

pub const BUNDLE_META_ENTRY: &str = "__soli_meta__";

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct BundleMeta {
    pub soli_version: String,
    pub ast_format: u8,
    pub protected: bool,
    /// Middleware entry path (`app/middleware/foo.sl`) → the
    /// `(function, order, global_only, scope_only)` directives that live in
    /// comments the AST does not keep.
    #[serde(default)]
    pub middleware: HashMap<String, Vec<(String, i32, bool, bool)>>,
    /// Pre-scanned controller registry info (actions, before/after hooks,
    /// layouts). NOTE: hook bodies ride along as small source snippets —
    /// they are executed per-request from source (documented v1 limitation).
    #[serde(default)]
    pub controllers: Vec<crate::interpreter::builtins::controller::controller::ControllerInfo>,
    /// Controller class → parent class, for hook/layout inheritance.
    #[serde(default)]
    pub controller_superclasses: HashMap<String, String>,
}

static BUNDLE_META: OnceLock<BundleMeta> = OnceLock::new();

/// The metadata of the bundle being served, if any (set once at boot).
pub fn bundle_meta() -> Option<&'static BundleMeta> {
    BUNDLE_META.get()
}

/// Validate the `__soli_meta__` entry of a bundle (if present) and stash it
/// for the serve pipeline. Protected bundles are locked to the exact soli
/// version that built them: the AST wire format has no cross-version
/// stability guarantee.
pub fn check_bundle_meta(entries: &[(String, &[u8])]) -> Result<(), String> {
    let Some((_, data)) = entries.iter().find(|(p, _)| p == BUNDLE_META_ENTRY) else {
        return Ok(()); // plain bundle
    };
    let meta: BundleMeta = serde_json::from_slice(data)
        .map_err(|e| format!("invalid {} entry: {}", BUNDLE_META_ENTRY, e))?;

    let running = env!("CARGO_PKG_VERSION");
    if meta.protected && (meta.soli_version != running || meta.ast_format != AST_FORMAT_VERSION) {
        return Err(format!(
            "this protected bundle was built with soli {} (AST format v{}) but the server runs \
             soli {} (AST format v{}) — rebuild the bundle with `soli build --protect`, or \
             install the matching soli version",
            meta.soli_version, meta.ast_format, running, AST_FORMAT_VERSION
        ));
    }

    let _ = BUNDLE_META.set(meta);
    Ok(())
}

/// Read and iterate entries from a serialized bundle.
pub struct BundleReader<'a> {
    _data: &'a [u8],
    entries: Vec<(String, &'a [u8])>,
}

impl<'a> BundleReader<'a> {
    pub fn new(data: &'a [u8]) -> Result<Self, String> {
        if is_encrypted_bundle(data) {
            return Err(ENCRYPTED_BUNDLE_HINT.to_string());
        }
        if data.len() < 8 || &data[..4] != BUNDLE_MAGIC {
            return Err("Invalid bundle: bad magic".to_string());
        }

        let count = u32::from_le_bytes([data[4], data[5], data[6], data[7]]) as usize;
        let mut offset = 8;
        let mut entries = Vec::with_capacity(count);

        for _ in 0..count {
            if offset + 4 > data.len() {
                return Err("Invalid bundle: truncated path length".to_string());
            }
            let path_len = u32::from_le_bytes([
                data[offset],
                data[offset + 1],
                data[offset + 2],
                data[offset + 3],
            ]) as usize;
            offset += 4;

            if offset
                .checked_add(path_len)
                .is_none_or(|end| end > data.len())
            {
                return Err("Invalid bundle: truncated path".to_string());
            }
            let path = std::str::from_utf8(&data[offset..offset + path_len])
                .map_err(|_| "Invalid bundle: non-UTF-8 path".to_string())?;
            validate_entry_path(path)?;
            offset += path_len;

            if offset + 8 > data.len() {
                return Err("Invalid bundle: truncated content length".to_string());
            }
            let content_len = u64::from_le_bytes([
                data[offset],
                data[offset + 1],
                data[offset + 2],
                data[offset + 3],
                data[offset + 4],
                data[offset + 5],
                data[offset + 6],
                data[offset + 7],
            ]) as usize;
            offset += 8;

            if offset
                .checked_add(content_len)
                .is_none_or(|end| end > data.len())
            {
                return Err("Invalid bundle: truncated content".to_string());
            }
            entries.push((path.to_string(), &data[offset..offset + content_len]));
            offset += content_len;
        }

        Ok(BundleReader {
            _data: data,
            entries,
        })
    }

    pub fn entries(&self) -> &[(String, &'a [u8])] {
        &self.entries
    }

    /// Look up one entry by exact path.
    pub fn get(&self, path: &str) -> Option<&'a [u8]> {
        self.entries
            .iter()
            .find(|(entry_path, _)| entry_path == path)
            .map(|(_, bytes)| *bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::virtual_fs::VirtualFileSystem;
    use std::fs;

    #[test]
    fn test_build_from_directory() {
        let dir = tempfile::tempdir().unwrap();
        let app_controllers = dir.path().join("app/controllers");
        fs::create_dir_all(&app_controllers).unwrap();
        fs::write(
            app_controllers.join("home_controller.sl"),
            b"class HomeController {}",
        )
        .unwrap();

        let config = dir.path().join("config");
        fs::create_dir_all(&config).unwrap();
        fs::write(config.join("routes.sl"), b"get('/') { \"hello\" }").unwrap();

        let bundle = BundleBuilder::build(dir.path()).unwrap();

        // Parse and verify
        let vfs = crate::virtual_fs::BundleFS::new(bundle).unwrap();
        assert!(vfs.exists("app/controllers/home_controller.sl"));
        assert!(vfs.exists("config/routes.sl"));
        assert_eq!(
            vfs.read_to_string("app/controllers/home_controller.sl")
                .unwrap(),
            "class HomeController {}"
        );
    }

    /// A bundle that carries the stylesheet but not the logo produces an app
    /// that boots and renders broken, with every asset 404 and nothing in the
    /// log to say so. Binary content must survive the round trip byte-for-byte.
    #[test]
    fn static_assets_are_bundled_alongside_code() {
        let dir = tempfile::tempdir().unwrap();
        let public = dir.path().join("public");
        fs::create_dir_all(&public).unwrap();

        // A PNG header: bytes that are deliberately not valid UTF-8.
        let png: &[u8] = &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0xFF, 0xFE];
        fs::write(public.join("icon-192.png"), png).unwrap();
        fs::write(public.join("offline.html"), b"<h1>offline</h1>").unwrap();
        fs::write(public.join("manifest.webmanifest"), b"{}").unwrap();
        fs::write(public.join("favicon.ico"), b"\x00\x00\x01\x00").unwrap();
        fs::write(public.join("app.woff2"), b"wOF2").unwrap();
        fs::write(public.join("notify.mp3"), b"ID3").unwrap();
        fs::write(dir.path().join("app.sl"), b"print(1)").unwrap();

        let bundle = BundleBuilder::build(dir.path()).unwrap();
        let vfs = crate::virtual_fs::BundleFS::new(bundle).unwrap();

        for asset in [
            "public/icon-192.png",
            "public/offline.html",
            "public/manifest.webmanifest",
            "public/favicon.ico",
            "public/app.woff2",
            "public/notify.mp3",
        ] {
            assert!(vfs.exists(asset), "{} should be bundled", asset);
        }
        assert_eq!(vfs.read("public/icon-192.png").unwrap(), png);
    }

    #[test]
    fn test_build_skips_hidden() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join(".hidden.sl"), b"secret").unwrap();
        fs::write(dir.path().join("visible.sl"), b"ok").unwrap();

        let bundle = BundleBuilder::build(dir.path()).unwrap();
        let vfs = crate::virtual_fs::BundleFS::new(bundle).unwrap();
        assert!(!vfs.exists(".hidden.sl"));
        assert!(vfs.exists("visible.sl"));
    }

    fn plain_fixture_bundle() -> Vec<u8> {
        let dir = tempfile::tempdir().unwrap();
        let controllers = dir.path().join("app/controllers");
        fs::create_dir_all(&controllers).unwrap();
        fs::write(
            controllers.join("home_controller.sl"),
            b"def index(req)\n  render(\"home/index\", {\"plans\": []})\nend\n",
        )
        .unwrap();
        BundleBuilder::build(dir.path()).unwrap()
    }

    #[test]
    fn encrypt_decrypt_round_trip() {
        let plain = plain_fixture_bundle();
        let sealed = encrypt_bundle(&plain, "round-trip-key").unwrap();
        assert!(is_encrypted_bundle(&sealed));
        assert_eq!(&sealed[..5], b"SOLE\x01");
        let back = decrypt_bundle(&sealed, "round-trip-key").unwrap();
        assert_eq!(back, plain);
        assert!(BundleReader::new(&back).is_ok());
    }

    #[test]
    fn decrypt_wrong_key_fails() {
        let sealed = encrypt_bundle(&plain_fixture_bundle(), "key-a").unwrap();
        let err = decrypt_bundle(&sealed, "key-b").unwrap_err();
        assert!(err.contains("wrong or rotated key"), "got: {err}");
    }

    #[test]
    fn decrypt_tampered_fails() {
        let mut sealed = encrypt_bundle(&plain_fixture_bundle(), "key").unwrap();
        let last = sealed.len() - 1;
        sealed[last] ^= 0xFF;
        assert!(decrypt_bundle(&sealed, "key").is_err());
    }

    #[test]
    fn decrypt_truncated_and_bad_version() {
        let err = decrypt_bundle(b"SOLE\x01short", "key").unwrap_err();
        assert!(err.contains("truncated"), "got: {err}");

        let mut sealed = encrypt_bundle(&plain_fixture_bundle(), "key").unwrap();
        sealed[4] = 9;
        let err = decrypt_bundle(&sealed, "key").unwrap_err();
        assert!(
            err.contains("unsupported encrypted bundle version"),
            "got: {err}"
        );
    }

    #[test]
    fn bundle_reader_rejects_encrypted_magic() {
        let sealed = encrypt_bundle(&plain_fixture_bundle(), "key").unwrap();
        let err = match BundleReader::new(&sealed) {
            Ok(_) => panic!("expected an error on encrypted magic"),
            Err(e) => e,
        };
        assert!(err.contains("encrypted"), "got: {err}");
        assert!(err.contains("SOLI_BUNDLE_KEY"), "got: {err}");
    }

    #[test]
    fn program_round_trips_through_slast_blob() {
        let program = parse_source(
            "class Greeter\n  def hello(name)\n    \"hi \" + name\n  end\nend\n\
             def index(req)\n  render(\"home/index\", {\"n\": 42})\nend\n",
        )
        .unwrap();
        let blob = serialize_program(&program).unwrap();
        assert!(is_ast_blob(&blob));
        let back = deserialize_program(&blob).unwrap();
        assert_eq!(back, program);
    }

    #[test]
    fn ast_blob_version_mismatch_rejected() {
        let program = parse_source("x = 1\n").unwrap();
        let mut blob = serialize_program(&program).unwrap();
        blob[5] = AST_FORMAT_VERSION + 1;
        let err = deserialize_program(&blob).unwrap_err();
        assert!(err.contains("rebuild the bundle"), "got: {err}");
    }

    #[test]
    fn build_protected_strips_source_and_adds_meta() {
        let dir = tempfile::tempdir().unwrap();
        let controllers = dir.path().join("app/controllers");
        fs::create_dir_all(&controllers).unwrap();
        fs::write(
            controllers.join("plans_controller.sl"),
            b"class PlansController < Controller\n  \
              # SECRET-COMMENT-TOKEN explains the trick\n  \
              def index(req)\n    render(\"plans/index\", {})\n  end\nend\n",
        )
        .unwrap();
        let middleware = dir.path().join("app/middleware");
        fs::create_dir_all(&middleware).unwrap();
        fs::write(
            middleware.join("auth.sl"),
            b"# order: 10\n# global_only: true\ndef check_auth(req)\n  req\nend\n",
        )
        .unwrap();

        let bundle = BundleBuilder::build_protected(dir.path()).unwrap();

        // The AST keeps identifiers/literals (like .pyc), but comments and
        // source syntax are gone: no comment text, no `def ` keyword text.
        let haystack = String::from_utf8_lossy(&bundle);
        assert!(!haystack.contains("SECRET-COMMENT-TOKEN"));
        assert!(!haystack.contains("def index"));

        let reader = BundleReader::new(&bundle).unwrap();
        let entries = reader.entries();

        let (_, controller) = entries
            .iter()
            .find(|(p, _)| p == "app/controllers/plans_controller.sl")
            .expect("controller entry");
        assert!(is_ast_blob(controller));
        assert!(deserialize_program(controller).is_ok());

        let (_, meta_bytes) = entries
            .iter()
            .find(|(p, _)| p == BUNDLE_META_ENTRY)
            .expect("meta entry");
        let meta: BundleMeta = serde_json::from_slice(meta_bytes).unwrap();
        assert!(meta.protected);
        assert_eq!(meta.soli_version, env!("CARGO_PKG_VERSION"));
        assert_eq!(meta.ast_format, AST_FORMAT_VERSION);
        // Middleware comment directives were precomputed.
        let directives = meta.middleware.get("app/middleware/auth.sl").unwrap();
        assert_eq!(
            directives,
            &vec![("check_auth".to_string(), 10, true, false)]
        );
        // Controller registry info was precomputed.
        assert_eq!(meta.controllers.len(), 1);
        assert_eq!(meta.controllers[0].name, "PlansController");
        assert_eq!(
            meta.controllers[0]
                .actions
                .iter()
                .map(|a| a.action_name.as_str())
                .collect::<Vec<_>>(),
            vec!["index"]
        );
    }

    #[test]
    fn check_bundle_meta_rejects_version_mismatch() {
        let meta = BundleMeta {
            soli_version: "0.0.1-not-this".to_string(),
            ast_format: AST_FORMAT_VERSION,
            protected: true,
            middleware: HashMap::new(),
            controllers: Vec::new(),
            controller_superclasses: HashMap::new(),
        };
        let json = serde_json::to_vec(&meta).unwrap();
        let entries = vec![(BUNDLE_META_ENTRY.to_string(), json.as_slice())];
        let err = check_bundle_meta(&entries).unwrap_err();
        assert!(err.contains("rebuild the bundle"), "got: {err}");
    }
}
