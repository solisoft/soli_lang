//! File I/O built-in functions.
//!
//! ## Filesystem jail (SEC-006)
//!
//! When the host enables a jail via [`set_file_jail`] (the production
//! `soli serve` startup does so unconditionally), every path passed to
//! the file builtins is resolved relative to the jail directory and
//! checked to remain *under* it. A controller calling
//! `File.read(req["params"]["path"])` therefore can no longer reach
//! `/etc/passwd` or escape the app root via `..` segments.
//!
//! `soli run`, the REPL, and the test runner do *not* set a jail, so
//! command-line scripts keep their full access to the local filesystem
//! exactly as before.
//!
//! ## Symlink defense (SEC-006a)
//!
//! On Unix, every `File.*` open also passes `O_NOFOLLOW`, and metadata
//! lookups go through `symlink_metadata` rather than `metadata`. That
//! closes a residual TOCTOU window where an attacker with local FS
//! access could swap a path with a symlink between `canonicalize` and
//! `open`. The `Trusted` class deliberately keeps following symlinks so
//! legitimate callers (tail a symlinked log file, read a config that
//! lives behind a `current` symlink) keep working.
//!
//! Code that genuinely needs to step outside the jail — log shippers,
//! backup scripts, cron-style maintenance jobs that ssh to other
//! machines — should call the parallel `Trusted` class
//! (`Trusted.read("/etc/...")`). It mirrors the `File` API but skips the
//! jail check **and** the `O_NOFOLLOW` flag.

use std::cell::RefCell;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::OnceLock;

use glob::Pattern;

use crate::interpreter::environment::Environment;
use crate::interpreter::value::{json_to_value, Class, NativeFunction, Value};

/// Process-wide filesystem jail. `None` means jail is disabled (CLI /
/// REPL / test runner). When `Some(path)`, every path that flows
/// through the `File`/standalone-function builtins must resolve to a
/// location under that path, with `..` segments rejected after
/// canonicalisation. The server installs this once at startup.
static FILE_JAIL: OnceLock<PathBuf> = OnceLock::new();

/// Install the filesystem jail. Idempotent on first call; subsequent
/// calls are no-ops because changing the jail mid-run would create a
/// race window where in-flight requests use the old root.
pub fn set_file_jail(path: PathBuf) {
    let _ = FILE_JAIL.set(path);
}

/// Internal accessor — `None` means "no jail enforced".
fn current_jail() -> Option<&'static Path> {
    FILE_JAIL.get().map(|p| p.as_path())
}

/// Resolve a user-supplied path, enforcing the jail when it is set.
///
/// - If the jail is unset: pass the path through unchanged.
/// - If the jail is set:
///   - Relative paths are joined onto the jail.
///   - The deepest existing ancestor is `canonicalize`d (so missing
///     leaf components are accepted, which is how `File.write` /
///     `mkdir_p` create new files), and the remaining tail is
///     re-attached.
///   - The result must `starts_with` the jail's canonicalised path.
///     Otherwise the call is rejected with `"<op>() path … escapes the
///     app-root jail"`.
fn resolve_path(path: &str, op: &str) -> Result<PathBuf, String> {
    resolve_with_jail(path, op, current_jail())
}

/// Pure helper exposed for unit tests — same logic as `resolve_path` but
/// takes the jail explicitly so tests can exercise it without mutating
/// the process-global `OnceLock`.
fn resolve_with_jail(path: &str, op: &str, jail: Option<&Path>) -> Result<PathBuf, String> {
    let jail = match jail {
        Some(j) => j,
        None => return Ok(PathBuf::from(path)),
    };
    let candidate = if Path::new(path).is_absolute() {
        PathBuf::from(path)
    } else {
        jail.join(path)
    };
    // Walk back to the deepest ancestor that actually exists, remember
    // every leaf segment we passed along the way, then canonicalise the
    // existing prefix and rebuild the full path. `canonicalize` requires
    // the leaf to exist, which would break `File.write("new.txt")`.
    let mut existing = candidate.clone();
    let mut tail: Vec<std::ffi::OsString> = Vec::new();
    while !existing.exists() {
        match (existing.file_name(), existing.parent()) {
            (Some(name), Some(parent)) => {
                tail.push(name.to_owned());
                existing = parent.to_path_buf();
            }
            _ => break,
        }
    }
    let canonical_prefix = fs::canonicalize(&existing)
        .map_err(|e| format!("{}() cannot resolve path {:?}: {}", op, path, e))?;
    let mut full = canonical_prefix;
    while let Some(part) = tail.pop() {
        full.push(part);
    }
    let canonical_jail = fs::canonicalize(jail).unwrap_or_else(|_| jail.to_path_buf());
    if !full.starts_with(&canonical_jail) {
        return Err(format!(
            "{}() path {:?} escapes the app-root jail at {}",
            op,
            path,
            canonical_jail.display()
        ));
    }
    Ok(full)
}

/// Resolve without jail enforcement, for the `Trusted` class. The
/// argument string is returned unchanged as a `PathBuf` so the call
/// sites stay symmetric with `resolve_path` and the operator
/// transparently sees the path they typed in error messages.
fn resolve_trusted(path: &str) -> PathBuf {
    PathBuf::from(path)
}

// ---------------------------------------------------------------------------
// Builtins — both standalone (`barf`, `slurp`, …) and the `File` class share
// the same per-op closures, parameterised over a small `FsPolicy` struct so
// the `Trusted` class can register the same set with the jail check skipped
// and symlink-following re-enabled.
// ---------------------------------------------------------------------------

#[derive(Clone, Copy)]
struct FsPolicy {
    /// Resolves user-supplied paths through the jail (or unchanged for
    /// `Trusted`).
    resolve: fn(&str, &str) -> Result<PathBuf, String>,
    /// `false` → `O_NOFOLLOW` on every open, `symlink_metadata` for
    /// metadata. `true` → vanilla open / `metadata`. The `Trusted` class
    /// uses `true` so callers can deliberately follow a symlink.
    follow_symlinks: bool,
}

const JAILED: FsPolicy = FsPolicy {
    resolve: jailed_resolver,
    follow_symlinks: false,
};

const TRUSTED: FsPolicy = FsPolicy {
    resolve: trusted_resolver,
    follow_symlinks: true,
};

fn jailed_resolver(path: &str, op: &str) -> Result<PathBuf, String> {
    resolve_path(path, op)
}

fn trusted_resolver(path: &str, _op: &str) -> Result<PathBuf, String> {
    Ok(resolve_trusted(path))
}

/// Apply `O_NOFOLLOW` to an `OpenOptions` builder when the policy says
/// not to follow symlinks. Compiled out on non-Unix targets — Windows
/// has no equivalent flag, so the jail check from SEC-006 is the only
/// protection there. (Soli's production deploy target is Linux, so the
/// Windows gap is documented but not a regression vs. pre-SEC-006a.)
#[cfg(unix)]
fn apply_nofollow(opts: &mut std::fs::OpenOptions, follow_symlinks: bool) {
    if !follow_symlinks {
        use std::os::unix::fs::OpenOptionsExt;
        opts.custom_flags(libc::O_NOFOLLOW);
    }
}

#[cfg(not(unix))]
fn apply_nofollow(_opts: &mut std::fs::OpenOptions, _follow_symlinks: bool) {}

/// Open a file for reading, honoring the policy's symlink stance.
fn open_for_read(path: &Path, follow_symlinks: bool) -> std::io::Result<std::fs::File> {
    let mut opts = std::fs::OpenOptions::new();
    opts.read(true);
    apply_nofollow(&mut opts, follow_symlinks);
    opts.open(path)
}

/// Open for write+create+truncate, honoring the policy.
fn open_for_write_truncate(path: &Path, follow_symlinks: bool) -> std::io::Result<std::fs::File> {
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    apply_nofollow(&mut opts, follow_symlinks);
    opts.open(path)
}

/// Open for create+append (the `File.append` builtin path).
///
/// SEC-050: append is the most footgun-y of the writes — without
/// `O_NOFOLLOW` an attacker who can plant a symlink at the target
/// (e.g. `app/log/app.log -> /etc/passwd`) turns a benign-looking
/// `File.append("log/app.log", line)` into a write to whatever the
/// link resolves to. The `apply_nofollow` call here is what closes
/// that path; `Trusted.append` deliberately keeps following so log
/// shippers can append to a `current` symlink.
fn open_for_append(path: &Path, follow_symlinks: bool) -> std::io::Result<std::fs::File> {
    let mut opts = std::fs::OpenOptions::new();
    opts.create(true).append(true);
    apply_nofollow(&mut opts, follow_symlinks);
    opts.open(path)
}

/// Read whole-file contents as a `String` through `open_for_read`.
fn read_to_string_policy(path: &Path, follow_symlinks: bool) -> std::io::Result<String> {
    use std::io::Read;
    let mut f = open_for_read(path, follow_symlinks)?;
    let mut s = String::new();
    f.read_to_string(&mut s)?;
    Ok(s)
}

/// Read whole-file contents as raw bytes through `open_for_read`.
fn read_to_bytes_policy(path: &Path, follow_symlinks: bool) -> std::io::Result<Vec<u8>> {
    use std::io::Read;
    let mut f = open_for_read(path, follow_symlinks)?;
    let mut buf = Vec::new();
    f.read_to_end(&mut buf)?;
    Ok(buf)
}

/// Write `bytes` to `path`, truncating any existing file.
fn write_all_policy(path: &Path, bytes: &[u8], follow_symlinks: bool) -> std::io::Result<()> {
    use std::io::Write;
    let mut f = open_for_write_truncate(path, follow_symlinks)?;
    f.write_all(bytes)
}

/// `metadata` if `follow_symlinks`, otherwise `symlink_metadata` so a
/// symlink reports the link's own type/size, not the target's.
fn metadata_policy(path: &Path, follow_symlinks: bool) -> std::io::Result<std::fs::Metadata> {
    if follow_symlinks {
        std::fs::metadata(path)
    } else {
        std::fs::symlink_metadata(path)
    }
}

/// Split `pattern_str` into the directory portion the caller should
/// resolve and read, and the basename portion to match against each
/// entry. An empty parent (relative pattern like `*.md`) becomes `.`
/// so the resolver lands at the cwd / jail root rather than failing on
/// `read_dir("")`.
fn split_glob_pattern(pattern_str: &str) -> (String, String) {
    let path = Path::new(pattern_str);
    let dir_str = match path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p.to_string_lossy().to_string(),
        _ => ".".to_string(),
    };
    let basename = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    (dir_str, basename)
}

/// Single-level glob. The caller supplies the resolver so the same
/// helper handles both the jailed `File.glob` and the trusted
/// `Trusted.glob` paths. BUG-002: the previous implementation built a
/// `Pattern` from the *full* user-supplied string and matched it
/// against bare basenames, which never succeeded for any pattern with
/// a `/`. Splitting the pattern first and matching only the basename
/// portion fixes that and makes relative patterns work too.
fn glob_paths<R>(pattern_str: &str, resolve: R) -> Result<Vec<PathBuf>, String>
where
    R: Fn(&str) -> Result<PathBuf, String>,
{
    let (dir_str, basename_pattern_str) = split_glob_pattern(pattern_str);
    let basename_pattern =
        Pattern::new(&basename_pattern_str).map_err(|e| format!("invalid pattern: {}", e))?;
    let resolved_dir = resolve(&dir_str)?;
    let entries =
        fs::read_dir(&resolved_dir).map_err(|e| format!("failed to read directory: {}", e))?;
    Ok(entries
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            let name = p
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            basename_pattern.matches(&name)
        })
        .collect())
}

/// Register all file I/O built-in functions.
pub fn register_file_builtins(env: &mut Environment) {
    define_standalone_file_builtins(env, JAILED);
    register_file_class(env, "File", JAILED);
    register_file_class(env, "Trusted", TRUSTED);
}

fn define_standalone_file_builtins(env: &mut Environment, policy: FsPolicy) {
    let resolve = policy.resolve;
    let follow = policy.follow_symlinks;
    // barf(path, content) - Write file (auto-detects text vs binary)
    env.define(
        "barf".to_string(),
        Value::NativeFunction(NativeFunction::new("barf", None, move |args| {
            match &args[..] {
                [Value::String(path), Value::String(content)] => {
                    let resolved = resolve(path, "barf")?;
                    write_all_policy(&resolved, content.as_bytes(), follow)
                        .map_err(|e| format!("barf failed to write {}: {}", path, e))?;
                    Ok(Value::Null)
                }
                [Value::String(path), Value::Array(bytes)] => {
                    let resolved = resolve(path, "barf")?;
                    let byte_vec: Result<Vec<u8>, String> = bytes
                        .borrow()
                        .iter()
                        .map(|b| match b {
                            Value::Int(n) if (0..=255).contains(n) => Ok(*n as u8),
                            Value::Int(n) => Err(format!("byte value {} out of range", n)),
                            other => Err(format!("expected byte, got {}", other.type_name())),
                        })
                        .collect();
                    write_all_policy(&resolved, &byte_vec?, follow)
                        .map_err(|e| format!("barf failed to write {}: {}", path, e))?;
                    Ok(Value::Null)
                }
                _ => Err("barf expects (string, string) or (string, array<int>)".to_string()),
            }
        })),
    );

    // slurp(path) or slurp(path, mode) - Read file (text or binary)
    env.define(
        "slurp".to_string(),
        Value::NativeFunction(NativeFunction::new(
            "slurp",
            None,
            move |args| match &args[..] {
                [Value::String(path)] => {
                    let resolved = resolve(path, "slurp")?;
                    read_to_string_policy(&resolved, follow)
                        .map(|s| Value::String(s.into()))
                        .map_err(|e| format!("slurp failed to read {}: {}", path, e))
                }
                [Value::String(path), Value::String(mode)] => {
                    let resolved = resolve(path, "slurp")?;
                    if **mode == *"binary" {
                        let bytes = read_to_bytes_policy(&resolved, follow)
                            .map_err(|e| format!("slurp failed to read {}: {}", path, e))?;
                        let value_bytes: Vec<Value> =
                            bytes.iter().map(|&b| Value::Int(b as i64)).collect();
                        Ok(Value::Array(Rc::new(RefCell::new(value_bytes))))
                    } else {
                        read_to_string_policy(&resolved, follow)
                            .map(|s| Value::String(s.into()))
                            .map_err(|e| format!("slurp failed to read {}: {}", path, e))
                    }
                }
                _ => Err("slurp expects path or (path, mode)".to_string()),
            },
        )),
    );

    // slurp_json(path) - Read and parse JSON file
    env.define(
        "slurp_json".to_string(),
        Value::NativeFunction(NativeFunction::new("slurp_json", Some(1), move |args| {
            let path = match &args[0] {
                Value::String(s) => s.clone(),
                _ => return Err("slurp_json expects a string path".to_string()),
            };
            let resolved = resolve(&path, "slurp_json")?;
            let content = read_to_string_policy(&resolved, follow)
                .map_err(|e| format!("slurp_json failed to read {}: {}", path, e))?;
            let json: serde_json::Value = serde_json::from_str(&content)
                .map_err(|e| format!("slurp_json failed to parse {}: {}", path, e))?;
            json_to_value(json)
        })),
    );

    // mkdir_p(path) - Create directory and all parent directories
    //
    // `create_dir_all` doesn't have a `nofollow` knob — it walks the
    // path and `mkdir`s each missing component. Since we already
    // canonicalised the deepest existing ancestor in `resolve_path`,
    // any symlink-out-of-jail in the existing prefix would have been
    // rejected before we get here.
    env.define(
        "mkdir_p".to_string(),
        Value::NativeFunction(NativeFunction::new("mkdir_p", Some(1), move |args| {
            let path = match &args[0] {
                Value::String(s) => s.clone(),
                _ => return Err("mkdir_p() expects string path".to_string()),
            };
            let resolved = resolve(&path, "mkdir_p")?;
            fs::create_dir_all(&resolved)
                .map(|_| Value::Bool(true))
                .map_err(|e| format!("mkdir_p() failed: {}", e))
        })),
    );

    // file_exists(path) - Check if file exists (standalone function)
    env.define(
        "file_exists".to_string(),
        Value::NativeFunction(NativeFunction::new("file_exists", Some(1), move |args| {
            let path = match &args[0] {
                Value::String(s) => s.clone(),
                _ => return Err("file_exists() expects string path".to_string()),
            };
            // For exists() the jail check still runs: an attacker shouldn't
            // be able to enumerate /etc by probing existence either. The
            // policy's symlink stance flows through too — `File.exists()`
            // on a symlink reports the link's existence, `Trusted.exists()`
            // follows the link.
            match resolve(&path, "file_exists") {
                Ok(resolved) => Ok(Value::Bool(metadata_policy(&resolved, follow).is_ok())),
                Err(_) => Ok(Value::Bool(false)),
            }
        })),
    );

    // file_write_base64(path, base64_data) - Decode base64 and write as binary
    env.define(
        "file_write_base64".to_string(),
        Value::NativeFunction(NativeFunction::new(
            "file_write_base64",
            Some(2),
            move |args| {
                let path = match &args[0] {
                    Value::String(s) => s.clone(),
                    _ => return Err("file_write_base64() expects string path".to_string()),
                };
                let data = match &args[1] {
                    Value::String(s) => s.clone(),
                    _ => return Err("file_write_base64() expects string data".to_string()),
                };
                let resolved = resolve(&path, "file_write_base64")?;
                use base64::Engine;
                let bytes = base64::engine::general_purpose::STANDARD
                    .decode(&*data)
                    .map_err(|e| format!("file_write_base64() decode failed: {}", e))?;
                write_all_policy(&resolved, &bytes, follow)
                    .map(|_| Value::Bool(true))
                    .map_err(|e| format!("file_write_base64() write failed: {}", e))
            },
        )),
    );

    // file_write_bytes(path, bytes) - Write raw bytes to file
    env.define(
        "file_write_bytes".to_string(),
        Value::NativeFunction(NativeFunction::new(
            "file_write_bytes",
            Some(2),
            move |args| {
                let path = match &args[0] {
                    Value::String(s) => s.clone(),
                    _ => return Err("file_write_bytes() expects string path".to_string()),
                };
                let bytes: Vec<u8> = match &args[1] {
                    Value::Array(arr) => arr
                        .borrow()
                        .iter()
                        .map(|v| match v {
                            Value::Int(n) if (0..=255).contains(n) => Ok(*n as u8),
                            _ => {
                                Err("file_write_bytes() expects array of bytes (0-255)".to_string())
                            }
                        })
                        .collect::<Result<Vec<u8>, String>>()?,
                    Value::String(s) => s.as_bytes().to_vec(),
                    _ => return Err("file_write_bytes() expects array or string data".to_string()),
                };
                let resolved = resolve(&path, "file_write_bytes")?;
                write_all_policy(&resolved, &bytes, follow)
                    .map(|_| Value::Bool(true))
                    .map_err(|e| format!("file_write_bytes() write failed: {}", e))
            },
        )),
    );
}

/// Register either the `File` (jailed + nofollow) or `Trusted`
/// (unjailed + follows symlinks) class.
fn register_file_class(env: &mut Environment, class_name: &'static str, policy: FsPolicy) {
    let resolve = policy.resolve;
    let follow = policy.follow_symlinks;
    let mut static_methods: HashMap<String, Rc<NativeFunction>> = HashMap::new();

    // File.read(path) - Read file contents as string
    {
        let label = format!("{}.read", class_name);
        static_methods.insert(
            "read".to_string(),
            Rc::new(NativeFunction::new(
                Box::leak(label.into_boxed_str()),
                Some(1),
                move |args| {
                    let path = match &args[0] {
                        Value::String(s) => s.clone(),
                        _ => return Err(format!("{}.read() expects string path", class_name)),
                    };
                    let resolved = resolve(&path, "read")?;
                    read_to_string_policy(&resolved, follow)
                        .map(|s| Value::String(s.into()))
                        .map_err(|e| format!("{}.read() failed: {}", class_name, e))
                },
            )),
        );
    }

    // File.write(path, content) - Write content to file
    {
        let label = format!("{}.write", class_name);
        static_methods.insert(
            "write".to_string(),
            Rc::new(NativeFunction::new(
                Box::leak(label.into_boxed_str()),
                Some(2),
                move |args| {
                    let path = match &args[0] {
                        Value::String(s) => s.clone(),
                        _ => return Err(format!("{}.write() expects string path", class_name)),
                    };
                    let content = match &args[1] {
                        Value::String(s) => s.clone(),
                        other => other.to_string().into(),
                    };
                    let resolved = resolve(&path, "write")?;
                    write_all_policy(&resolved, content.as_bytes(), follow)
                        .map(|_| Value::Bool(true))
                        .map_err(|e| format!("{}.write() failed: {}", class_name, e))
                },
            )),
        );
    }

    // File.exists(path) - Check if file exists
    {
        let label = format!("{}.exists", class_name);
        static_methods.insert(
            "exists".to_string(),
            Rc::new(NativeFunction::new(
                Box::leak(label.into_boxed_str()),
                Some(1),
                move |args| {
                    let path = match &args[0] {
                        Value::String(s) => s.clone(),
                        _ => return Err(format!("{}.exists() expects string path", class_name)),
                    };
                    // Use metadata_policy so File.exists("symlink") reports
                    // the symlink itself (true even if the target is gone),
                    // while Trusted.exists() reports the target.
                    match resolve(&path, "exists") {
                        Ok(resolved) => Ok(Value::Bool(metadata_policy(&resolved, follow).is_ok())),
                        Err(_) => Ok(Value::Bool(false)),
                    }
                },
            )),
        );
    }

    // File.delete(path) - Delete a file
    {
        let label = format!("{}.delete", class_name);
        static_methods.insert(
            "delete".to_string(),
            Rc::new(NativeFunction::new(
                Box::leak(label.into_boxed_str()),
                Some(1),
                move |args| {
                    let path = match &args[0] {
                        Value::String(s) => s.clone(),
                        _ => return Err(format!("{}.delete() expects string path", class_name)),
                    };
                    let resolved = resolve(&path, "delete")?;
                    fs::remove_file(&resolved)
                        .map(|_| Value::Bool(true))
                        .map_err(|e| format!("{}.delete() failed: {}", class_name, e))
                },
            )),
        );
    }

    // File.is_file(path) — uses metadata_policy so a symlink reports
    // false under `File.*` (the link itself is not a regular file under
    // symlink_metadata) and follows the link under `Trusted.*`.
    {
        let label = format!("{}.is_file", class_name);
        static_methods.insert(
            "is_file".to_string(),
            Rc::new(NativeFunction::new(
                Box::leak(label.into_boxed_str()),
                Some(1),
                move |args| {
                    let path = match &args[0] {
                        Value::String(s) => s.clone(),
                        _ => return Err(format!("{}.is_file() expects string path", class_name)),
                    };
                    match resolve(&path, "is_file") {
                        Ok(resolved) => match metadata_policy(&resolved, follow) {
                            Ok(m) => Ok(Value::Bool(m.is_file())),
                            Err(_) => Ok(Value::Bool(false)),
                        },
                        Err(_) => Ok(Value::Bool(false)),
                    }
                },
            )),
        );
    }

    // File.is_dir(path)
    {
        let label = format!("{}.is_dir", class_name);
        static_methods.insert(
            "is_dir".to_string(),
            Rc::new(NativeFunction::new(
                Box::leak(label.into_boxed_str()),
                Some(1),
                move |args| {
                    let path = match &args[0] {
                        Value::String(s) => s.clone(),
                        _ => return Err(format!("{}.is_dir() expects string path", class_name)),
                    };
                    match resolve(&path, "is_dir") {
                        Ok(resolved) => match metadata_policy(&resolved, follow) {
                            Ok(m) => Ok(Value::Bool(m.is_dir())),
                            Err(_) => Ok(Value::Bool(false)),
                        },
                        Err(_) => Ok(Value::Bool(false)),
                    }
                },
            )),
        );
    }

    // File.size(path)
    {
        let label = format!("{}.size", class_name);
        static_methods.insert(
            "size".to_string(),
            Rc::new(NativeFunction::new(
                Box::leak(label.into_boxed_str()),
                Some(1),
                move |args| {
                    let path = match &args[0] {
                        Value::String(s) => s.clone(),
                        _ => return Err(format!("{}.size() expects string path", class_name)),
                    };
                    let resolved = resolve(&path, "size")?;
                    metadata_policy(&resolved, follow)
                        .map(|m| Value::Int(m.len() as i64))
                        .map_err(|e| format!("{}.size() failed: {}", class_name, e))
                },
            )),
        );
    }

    // File.modified(path)
    {
        let label = format!("{}.modified", class_name);
        static_methods.insert(
            "modified".to_string(),
            Rc::new(NativeFunction::new(
                Box::leak(label.into_boxed_str()),
                Some(1),
                move |args| {
                    let path = match &args[0] {
                        Value::String(s) => s.clone(),
                        _ => return Err(format!("{}.modified() expects string path", class_name)),
                    };
                    let resolved = resolve(&path, "modified")?;
                    metadata_policy(&resolved, follow)
                        .and_then(|m| m.modified())
                        .map(|t| {
                            let duration = t.duration_since(std::time::UNIX_EPOCH).unwrap();
                            Value::Int(duration.as_secs() as i64)
                        })
                        .map_err(|e| format!("{}.modified() failed: {}", class_name, e))
                },
            )),
        );
    }

    // File.append(path, content)
    {
        let label = format!("{}.append", class_name);
        static_methods.insert(
            "append".to_string(),
            Rc::new(NativeFunction::new(
                Box::leak(label.into_boxed_str()),
                Some(2),
                move |args| {
                    use std::io::Write;
                    let path = match &args[0] {
                        Value::String(s) => s.clone(),
                        _ => return Err(format!("{}.append() expects string path", class_name)),
                    };
                    let content = match &args[1] {
                        Value::String(s) => s.clone(),
                        other => other.to_string().into(),
                    };
                    let resolved = resolve(&path, "append")?;
                    let mut file = open_for_append(&resolved, follow)
                        .map_err(|e| format!("{}.append() failed to open: {}", class_name, e))?;
                    file.write_all(content.as_bytes())
                        .map(|_| Value::Bool(true))
                        .map_err(|e| format!("{}.append() failed to write: {}", class_name, e))
                },
            )),
        );
    }

    // File.lines(path)
    {
        let label = format!("{}.lines", class_name);
        static_methods.insert(
            "lines".to_string(),
            Rc::new(NativeFunction::new(
                Box::leak(label.into_boxed_str()),
                Some(1),
                move |args| {
                    let path = match &args[0] {
                        Value::String(s) => s.clone(),
                        _ => return Err(format!("{}.lines() expects string path", class_name)),
                    };
                    let resolved = resolve(&path, "lines")?;
                    let content = read_to_string_policy(&resolved, follow)
                        .map_err(|e| format!("{}.lines() failed: {}", class_name, e))?;
                    let lines: Vec<Value> = content
                        .lines()
                        .map(|l| Value::String(l.to_string().into()))
                        .collect();
                    Ok(Value::Array(Rc::new(RefCell::new(lines))))
                },
            )),
        );
    }

    // File.copy(src, dest) — read src + write dest through the policy so
    // a symlinked src under `File.copy` cannot smuggle in
    // attacker-controlled content from outside the jail.
    {
        let label = format!("{}.copy", class_name);
        static_methods.insert(
            "copy".to_string(),
            Rc::new(NativeFunction::new(
                Box::leak(label.into_boxed_str()),
                Some(2),
                move |args| {
                    let src = match &args[0] {
                        Value::String(s) => s.clone(),
                        _ => {
                            return Err(format!("{}.copy() expects string source path", class_name))
                        }
                    };
                    let dest = match &args[1] {
                        Value::String(s) => s.clone(),
                        _ => {
                            return Err(format!(
                                "{}.copy() expects string destination path",
                                class_name
                            ))
                        }
                    };
                    let src_resolved = resolve(&src, "copy")?;
                    let dest_resolved = resolve(&dest, "copy")?;
                    let mut src_f = open_for_read(&src_resolved, follow)
                        .map_err(|e| format!("{}.copy() failed: {}", class_name, e))?;
                    let mut dest_f = open_for_write_truncate(&dest_resolved, follow)
                        .map_err(|e| format!("{}.copy() failed: {}", class_name, e))?;
                    std::io::copy(&mut src_f, &mut dest_f)
                        .map(|_| Value::Bool(true))
                        .map_err(|e| format!("{}.copy() failed: {}", class_name, e))
                },
            )),
        );
    }

    // File.rename(old_path, new_path)
    {
        let label = format!("{}.rename", class_name);
        static_methods.insert(
            "rename".to_string(),
            Rc::new(NativeFunction::new(
                Box::leak(label.into_boxed_str()),
                Some(2),
                move |args| {
                    let old_path = match &args[0] {
                        Value::String(s) => s.clone(),
                        _ => {
                            return Err(format!("{}.rename() expects string old path", class_name))
                        }
                    };
                    let new_path = match &args[1] {
                        Value::String(s) => s.clone(),
                        _ => {
                            return Err(format!("{}.rename() expects string new path", class_name))
                        }
                    };
                    let old_resolved = resolve(&old_path, "rename")?;
                    let new_resolved = resolve(&new_path, "rename")?;
                    fs::rename(&old_resolved, &new_resolved)
                        .map(|_| Value::Bool(true))
                        .map_err(|e| format!("{}.rename() failed: {}", class_name, e))
                },
            )),
        );
    }

    // File.glob(pattern)
    {
        let label = format!("{}.glob", class_name);
        static_methods.insert(
            "glob".to_string(),
            Rc::new(NativeFunction::new(
                Box::leak(label.into_boxed_str()),
                Some(1),
                move |args| {
                    let pattern_str = match &args[0] {
                        Value::String(s) => s.clone(),
                        _ => return Err(format!("{}.glob() expects string pattern", class_name)),
                    };
                    let paths = glob_paths(&pattern_str, |dir| resolve(dir, "glob"))
                        .map_err(|e| format!("{}.glob() {}", class_name, e))?;
                    let matches: Vec<Value> = paths
                        .into_iter()
                        .map(|p| Value::String(p.to_string_lossy().to_string().into()))
                        .collect();
                    Ok(Value::Array(Rc::new(RefCell::new(matches))))
                },
            )),
        );
    }

    // File.glob_recursive(pattern)
    {
        let label = format!("{}.glob_recursive", class_name);
        static_methods.insert(
            "glob_recursive".to_string(),
            Rc::new(NativeFunction::new(
                Box::leak(label.into_boxed_str()),
                Some(1),
                move |args| {
                    let pattern_str = match &args[0] {
                        Value::String(s) => s.clone(),
                        _ => {
                            return Err(format!(
                                "{}.glob_recursive() expects string pattern",
                                class_name
                            ))
                        }
                    };
                    let pattern = Pattern::new(&pattern_str).map_err(|e| {
                        format!("{}.glob_recursive() invalid pattern: {}", class_name, e)
                    })?;
                    let path = Path::new(&*pattern_str);
                    let base_dir_str = path
                        .parent()
                        .map(|p| p.to_string_lossy().to_string())
                        .unwrap_or(".".to_string());
                    let resolved_base = resolve(&base_dir_str, "glob_recursive")?;
                    let mut matches: Vec<Value> = Vec::new();
                    for entry in walkdir::WalkDir::new(&resolved_base)
                        .into_iter()
                        .filter_map(|e| e.ok())
                    {
                        let path_str = entry.path().to_string_lossy().to_string();
                        let base_str = resolved_base.to_string_lossy().to_string();
                        let relative = if path_str.starts_with(&base_str) {
                            let after = &path_str[base_str.len()..];
                            if after.starts_with('/') || after.starts_with('\\') {
                                after[1..].to_string()
                            } else {
                                after.to_string()
                            }
                        } else {
                            path_str.clone()
                        };
                        if pattern.matches(&relative) || pattern.matches(&path_str) {
                            matches.push(Value::String(path_str.into()));
                        }
                    }
                    Ok(Value::Array(Rc::new(RefCell::new(matches))))
                },
            )),
        );
    }

    let class = Class {
        name: class_name.to_string(),
        superclass: None,
        methods: Rc::new(RefCell::new(HashMap::new())),
        static_methods: HashMap::new(),
        native_static_methods: static_methods,
        native_methods: HashMap::new(),
        static_fields: Rc::new(RefCell::new(HashMap::new())),
        fields: HashMap::new(),
        constructor: None,
        nested_classes: Rc::new(RefCell::new(HashMap::new())),
        ..Default::default()
    };

    env.define(class_name.to_string(), Value::Class(Rc::new(class)));
}

#[cfg(test)]
mod tests {
    //! Unit tests for the SEC-006 jail. The OnceLock-based global is
    //! awkward to mutate from tests, so we exercise the pure helper
    //! `resolve_with_jail` and pass the jail in explicitly.
    use super::*;

    #[test]
    fn no_jail_passes_paths_through_unchanged() {
        let resolved = resolve_with_jail("/etc/passwd", "read", None).unwrap();
        assert_eq!(resolved, PathBuf::from("/etc/passwd"));
    }

    #[test]
    fn jailed_relative_path_resolves_under_jail() {
        let dir = tempfile::tempdir().unwrap();
        let jail = dir.path();
        std::fs::write(jail.join("inside.txt"), "ok").unwrap();
        let resolved = resolve_with_jail("inside.txt", "read", Some(jail)).unwrap();
        assert!(resolved.starts_with(std::fs::canonicalize(jail).unwrap()));
        assert!(resolved.ends_with("inside.txt"));
    }

    #[test]
    fn jailed_absolute_path_outside_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let jail = dir.path();
        let err = resolve_with_jail("/etc/passwd", "read", Some(jail)).unwrap_err();
        assert!(err.contains("escapes the app-root jail"), "{}", err);
    }

    #[test]
    fn jailed_dot_dot_escape_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let jail = dir.path().join("project");
        std::fs::create_dir(&jail).unwrap();
        std::fs::write(dir.path().join("outside.txt"), "x").unwrap();
        let err = resolve_with_jail("../outside.txt", "read", Some(&jail)).unwrap_err();
        assert!(err.contains("escapes the app-root jail"), "{}", err);
    }

    #[test]
    fn jailed_symlink_escape_is_rejected_after_canonicalisation() {
        // Symlink leaves the jail; canonicalisation should follow it and
        // the resulting path must fail the `starts_with` check.
        let outer = tempfile::tempdir().unwrap();
        let jail = outer.path().join("project");
        std::fs::create_dir(&jail).unwrap();
        let target = outer.path().join("secret.txt");
        std::fs::write(&target, "leak").unwrap();
        let link = jail.join("link");
        // Skip on platforms without symlink support.
        if std::os::unix::fs::symlink(&target, &link).is_err() {
            return;
        }
        let err = resolve_with_jail("link", "read", Some(&jail)).unwrap_err();
        assert!(err.contains("escapes the app-root jail"), "{}", err);
    }

    #[test]
    fn jail_allows_creating_a_new_file_inside() {
        // The leaf path doesn't exist yet (this is the `File.write
        // "new.txt"` shape). Resolution must succeed and produce a path
        // under the jail.
        let dir = tempfile::tempdir().unwrap();
        let jail = dir.path();
        let resolved = resolve_with_jail("subdir/new.txt", "write", Some(jail)).unwrap();
        // Build the expected canonical jail path manually to avoid
        // relying on whether the leaf exists.
        assert!(resolved.starts_with(std::fs::canonicalize(jail).unwrap()));
    }

    #[test]
    fn trusted_resolver_skips_jail() {
        // Sanity check: the `Trusted` class's resolver returns the path
        // unchanged regardless of jail state.
        let p = trusted_resolver("/anywhere", "read").unwrap();
        assert_eq!(p, PathBuf::from("/anywhere"));
    }

    // ----- SEC-006a: O_NOFOLLOW / symlink_metadata behaviour ---------------

    #[cfg(unix)]
    #[test]
    fn nofollow_append_rejects_symlink() {
        // SEC-050: open_for_append must refuse to follow a symlink under
        // the jailed policy. Without O_NOFOLLOW, a planted
        // `app/log/app.log -> /etc/passwd` symlink would let
        // `File.append("log/app.log", line)` write into whatever the
        // link points at.
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("real.log");
        std::fs::write(&target, "ok\n").unwrap();
        let link = dir.path().join("link.log");
        if std::os::unix::fs::symlink(&target, &link).is_err() {
            return;
        }
        let err = open_for_append(&link, /* follow */ false).unwrap_err();
        assert_eq!(
            err.raw_os_error(),
            Some(libc::ELOOP),
            "expected ELOOP, got {:?}",
            err
        );
    }

    #[cfg(unix)]
    #[test]
    fn follow_append_succeeds_through_symlink_for_trusted() {
        // Trusted.append deliberately follows symlinks so log shippers
        // can target a `current` link. Sanity check the escape hatch.
        use std::io::Write;
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("real.log");
        std::fs::write(&target, "header\n").unwrap();
        let link = dir.path().join("link.log");
        if std::os::unix::fs::symlink(&target, &link).is_err() {
            return;
        }
        let mut f = open_for_append(&link, /* follow */ true).unwrap();
        f.write_all(b"more\n").unwrap();
        drop(f);
        let body = std::fs::read_to_string(&target).unwrap();
        assert_eq!(body, "header\nmore\n");
    }

    #[cfg(unix)]
    #[test]
    fn nofollow_open_rejects_symlink_even_when_target_is_in_jail() {
        // Even with the symlink target inside the jail, `O_NOFOLLOW`
        // must refuse to open through the link. This is the regression
        // SEC-006a defends against (a local TOCTOU swap that points an
        // already-canonicalised path at a symlink before the open).
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("real.txt");
        std::fs::write(&target, "ok").unwrap();
        let link = dir.path().join("link.txt");
        if std::os::unix::fs::symlink(&target, &link).is_err() {
            return;
        }
        let err = open_for_read(&link, /* follow */ false).unwrap_err();
        assert_eq!(
            err.raw_os_error(),
            Some(libc::ELOOP),
            "expected ELOOP, got {:?}",
            err
        );
    }

    #[cfg(unix)]
    #[test]
    fn follow_open_succeeds_through_symlink_for_trusted() {
        // The `Trusted` policy keeps following symlinks; this is the
        // legitimate "tail a symlinked log file" use case.
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("real.txt");
        std::fs::write(&target, "hello").unwrap();
        let link = dir.path().join("link.txt");
        if std::os::unix::fs::symlink(&target, &link).is_err() {
            return;
        }
        let mut s = String::new();
        use std::io::Read;
        open_for_read(&link, /* follow */ true)
            .unwrap()
            .read_to_string(&mut s)
            .unwrap();
        assert_eq!(s, "hello");
    }

    // ----- BUG-002: glob basename matching ---------------------------------

    #[test]
    fn split_glob_pattern_extracts_dir_and_basename() {
        assert_eq!(
            split_glob_pattern("/abs/path/*"),
            ("/abs/path".to_string(), "*".to_string())
        );
        assert_eq!(
            split_glob_pattern("/abs/path/*.md"),
            ("/abs/path".to_string(), "*.md".to_string())
        );
        assert_eq!(
            split_glob_pattern("subdir/*.md"),
            ("subdir".to_string(), "*.md".to_string())
        );
    }

    #[test]
    fn split_glob_pattern_uses_dot_for_relative_pattern_without_parent() {
        // BUG-002 repro: parent() of `*.md` is `Some("")`, which used
        // to flow through to `read_dir("")` and silently fail.
        assert_eq!(
            split_glob_pattern("*.md"),
            (".".to_string(), "*.md".to_string())
        );
        assert_eq!(split_glob_pattern("*"), (".".to_string(), "*".to_string()));
    }

    #[test]
    fn glob_paths_absolute_pattern_with_star_returns_children() {
        // Direct repro of the BUG-002 scenario: an absolute pattern
        // ending in `/*` must enumerate the directory's children.
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.md"), "").unwrap();
        std::fs::write(dir.path().join("b.txt"), "").unwrap();
        std::fs::create_dir(dir.path().join("sub")).unwrap();
        let pattern = format!("{}/*", dir.path().display());
        let paths = glob_paths(&pattern, |d| Ok(PathBuf::from(d))).unwrap();
        let mut names: Vec<String> = paths
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        names.sort();
        assert_eq!(names, vec!["a.md", "b.txt", "sub"]);
    }

    #[test]
    fn glob_paths_absolute_pattern_filters_by_extension() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.md"), "").unwrap();
        std::fs::write(dir.path().join("b.md"), "").unwrap();
        std::fs::write(dir.path().join("c.txt"), "").unwrap();
        let pattern = format!("{}/*.md", dir.path().display());
        let paths = glob_paths(&pattern, |d| Ok(PathBuf::from(d))).unwrap();
        let mut names: Vec<String> = paths
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        names.sort();
        assert_eq!(names, vec!["a.md", "b.md"]);
    }

    #[test]
    fn glob_paths_relative_pattern_resolves_under_jail() {
        // Covers the `File.glob` (jailed) class. A relative pattern
        // `*.md` lands at the jail root via `split_glob_pattern`'s
        // empty-parent → "." substitution.
        let dir = tempfile::tempdir().unwrap();
        let jail = dir.path().to_path_buf();
        std::fs::write(jail.join("a.md"), "").unwrap();
        std::fs::write(jail.join("b.md"), "").unwrap();
        std::fs::write(jail.join("c.txt"), "").unwrap();
        let paths = glob_paths("*.md", |d| resolve_with_jail(d, "glob", Some(&jail))).unwrap();
        let mut names: Vec<String> = paths
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        names.sort();
        assert_eq!(names, vec!["a.md", "b.md"]);
    }

    #[test]
    fn glob_paths_handles_deeply_nested_path() {
        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("a/b/c");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(nested.join("file.txt"), "").unwrap();
        std::fs::write(nested.join("note.md"), "").unwrap();
        let pattern = format!("{}/*.md", nested.display());
        let paths = glob_paths(&pattern, |d| Ok(PathBuf::from(d))).unwrap();
        let names: Vec<String> = paths
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, vec!["note.md"]);
    }

    #[test]
    fn glob_paths_uses_trusted_resolver_for_absolute_pattern() {
        // Covers the `Trusted.glob` (unjailed) class via the same
        // resolver the production registration uses.
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("alpha.txt"), "").unwrap();
        std::fs::write(dir.path().join("beta.txt"), "").unwrap();
        let pattern = format!("{}/*", dir.path().display());
        let paths = glob_paths(&pattern, |d| trusted_resolver(d, "glob")).unwrap();
        let mut names: Vec<String> = paths
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        names.sort();
        assert_eq!(names, vec!["alpha.txt", "beta.txt"]);
    }

    #[cfg(unix)]
    #[test]
    fn metadata_policy_returns_link_under_nofollow_target_under_follow() {
        // With follow=false, metadata_policy reports the symlink itself
        // (file_type().is_symlink() == true). With follow=true, it
        // reports the regular-file target.
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("real.txt");
        std::fs::write(&target, "x").unwrap();
        let link = dir.path().join("link.txt");
        if std::os::unix::fs::symlink(&target, &link).is_err() {
            return;
        }
        let link_meta = metadata_policy(&link, false).unwrap();
        assert!(link_meta.file_type().is_symlink());
        let target_meta = metadata_policy(&link, true).unwrap();
        assert!(target_meta.is_file() && !target_meta.file_type().is_symlink());
    }
}
