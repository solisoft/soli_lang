//! `eui_window(name, handler, view, options)`: a script that opens its own EUI
//! window.
//!
//! An EUI view needs a server — the `/_eui/session/<name>` socket of
//! `soli serve` — and a client to draw it. A script has neither, so this
//! builds both around it, the way a `soli desktop build --eui` artifact does
//! at launch:
//!
//! 1. The script's *definitions* (functions, classes, enums, constants) are
//!    written into a throw-away application, as a serialized AST, beside the
//!    widget catalogue and a `config/routes.sl` that registers the component.
//!    The server's realtime
//!    workers run in their own interpreters and cannot see the script's;
//!    loading the same definitions is how they find the handler and the view.
//!    The script's top-level statements are not part of it — they run once,
//!    in the script — so a handler keeps what it needs in the EUI state, not
//!    in a top-level variable.
//! 2. That application is served on a background thread, on `127.0.0.1` and a
//!    port the OS picks, behind the desktop loopback gate: only the window,
//!    which holds the session cookie, can open the socket.
//! 3. The window opens on the calling thread — the event loop must own the
//!    main thread — and `eui_window` returns when it is closed.
//!
//! The window itself is the `eui-desktop` feature. A soli built without it
//! raises a clear error; `SOLI_EUI_NO_WINDOW=1` serves and prints the session
//! URL and cookie instead, for a client started by hand or a headless test.

use std::collections::HashSet;
use std::sync::Mutex;

use crate::ast::{Program, Stmt, StmtKind};
use crate::interpreter::environment::Environment;
use crate::interpreter::value::{HashKey, NativeFunction, Value};

/// The definitions of the script being run, kept when it calls `eui_window`.
static DEFINITIONS: Mutex<Option<Program>> = Mutex::new(None);

/// Environment variable: serve and print the session instead of opening a
/// window.
pub const NO_WINDOW_ENV: &str = "SOLI_EUI_NO_WINDOW";

/// Keep `program`'s definitions for `eui_window`, if it calls it. Called once
/// per script before it runs; a script that never mentions `eui_window` costs
/// one walk of its AST.
pub fn remember_script(program: &Program) {
    if !mentions_eui_window(program) {
        return;
    }
    let definitions = program
        .statements
        .iter()
        .filter(|stmt| is_definition(stmt))
        .cloned()
        .collect();
    if let Ok(mut slot) = DEFINITIONS.lock() {
        *slot = Some(Program::new(definitions));
    }
}

pub(crate) fn mentions_eui_window(program: &Program) -> bool {
    let mut found = false;
    crate::ast::walk::walk_program(program, &mut |_| {}, &mut |expr| {
        if let crate::ast::ExprKind::Variable(name) = &expr.kind {
            if name == "eui_window" {
                found = true;
            }
        }
    });
    found
}

fn is_definition(stmt: &Stmt) -> bool {
    match &stmt.kind {
        StmtKind::Function(_)
        | StmtKind::Class(_)
        | StmtKind::Enum(_)
        | StmtKind::Interface(_)
        | StmtKind::Const { .. } => true,
        StmtKind::Export(inner) => is_definition(inner),
        _ => false,
    }
}

fn defined_names(program: &Program) -> HashSet<String> {
    fn name_of(stmt: &Stmt, out: &mut HashSet<String>) {
        match &stmt.kind {
            StmtKind::Function(decl) => {
                out.insert(decl.name.clone());
            }
            StmtKind::Export(inner) => name_of(inner, out),
            _ => {}
        }
    }
    let mut names = HashSet::new();
    for stmt in &program.statements {
        name_of(stmt, &mut names);
    }
    names
}

/// A component name and the two function names go into a route string and a
/// URL: keep them to what an identifier may contain.
fn plain_name(value: &str) -> bool {
    !value.is_empty()
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '?')
}

pub fn register_eui_window(env: &mut Environment) {
    env.define(
        "eui_window".to_string(),
        Value::NativeFunction(NativeFunction::new("eui_window", None, |args| {
            if !(3..=4).contains(&args.len()) {
                return Err(
                    "eui_window(name, handler, view, options?) takes 3 or 4 arguments".to_string(),
                );
            }
            let text = |i: usize, what: &str| match &args[i] {
                Value::String(s) if plain_name(s) => Ok(s.to_string()),
                Value::String(s) => Err(format!(
                    "eui_window: the {what} {s:?} must be a plain name (letters, digits, _ or -)"
                )),
                other => Err(format!(
                    "eui_window: the {what} must be a String, got {}",
                    other.type_name()
                )),
            };
            let name = text(0, "component name")?;
            let handler = text(1, "handler")?;
            let view = text(2, "view")?;
            let title = match args.get(3) {
                None | Some(Value::Null) => name.clone(),
                Some(Value::Hash(pairs)) => {
                    let mut title = name.clone();
                    for (key, value) in pairs.borrow().iter() {
                        match (key, value) {
                            (HashKey::String(k), Value::String(v)) if &**k == "title" => {
                                title = v.to_string();
                            }
                            (other, _) => {
                                return Err(format!(
                                    "eui_window: unknown option {other} (expected \"title\")"
                                ))
                            }
                        }
                    }
                    title
                }
                Some(other) => {
                    return Err(format!(
                        "eui_window: options must be a Hash, got {}",
                        other.type_name()
                    ))
                }
            };
            open(&name, &handler, &view, &title)?;
            Ok(Value::Null)
        })),
    );
}

fn open(name: &str, handler: &str, view: &str, title: &str) -> Result<(), String> {
    let definitions = DEFINITIONS
        .lock()
        .ok()
        .and_then(|slot| slot.clone())
        .ok_or_else(|| {
            "eui_window opens a window from a script run with `soli app.sl`".to_string()
        })?;
    let names = defined_names(&definitions);
    for (what, function) in [("handler", handler), ("view", view)] {
        if !names.contains(function) {
            return Err(format!(
                "eui_window: the {what} `{function}` is not a top-level `def` of the script"
            ));
        }
    }

    let headless = std::env::var_os(NO_WINDOW_ENV).is_some_and(|v| !v.is_empty() && v != "0");
    if !headless && !cfg!(feature = "eui-desktop") {
        return Err(format!(
            "eui_window: this soli has no built-in EUI window (it was built without the \
             `eui-desktop` feature). Build it with `--features eui-desktop`, or set \
             {NO_WINDOW_ENV}=1 to serve the session and open it with an EUI client."
        ));
    }

    let app = write_app(&definitions, name, handler, view)?;
    let (port, session) = serve(&app.path().join(name))?;
    let url = format!("ws://127.0.0.1:{port}/_eui/session/{name}");
    let cookie = crate::desktop::token::cookie_header_value(&session);

    if headless {
        println!("{url}");
        println!("{cookie}");
        // Served until the process is stopped: the window is someone else's.
        loop {
            std::thread::sleep(std::time::Duration::from_secs(3600));
        }
    }
    let result = window(url, cookie, title);
    // The temporary application goes with the window; the server thread
    // stops with the process.
    drop(app);
    result
}

/// The throw-away application the server loads: the widget catalogue, the
/// definitions as a controller file, the component's route, and the
/// directories a Soli app is expected to have.
fn write_app(
    definitions: &Program,
    name: &str,
    handler: &str,
    view: &str,
) -> Result<tempfile::TempDir, String> {
    let dir = tempfile::Builder::new()
        .prefix("soli-eui-")
        .tempdir()
        .map_err(|e| format!("eui_window: cannot create a temporary directory: {e}"))?;
    // The application is named after its folder, and the client pins the
    // publisher key per application name: the component's name, not the
    // temporary directory's, so every run is the same application.
    let root = &dir.path().join(name);
    let make = |sub: &str| {
        std::fs::create_dir_all(root.join(sub))
            .map_err(|e| format!("eui_window: cannot create {sub}: {e}"))
    };
    make("app/controllers")?;
    make("app/models")?;
    make("config")?;
    // The widget catalogue a `soli new --eui` application gets, so a view
    // can say `column`, `button` or `tw(...)`. Controllers load in name
    // order, `script.sl` after these: a script's own `def text` wins.
    for (file, source) in crate::scaffold::templates::eui::EUI_BUILDERS {
        std::fs::write(root.join("app/controllers").join(file), source)
            .map_err(|e| format!("eui_window: cannot write {file}: {e}"))?;
    }
    let blob = crate::bundle::serialize_program(definitions)?;
    std::fs::write(root.join("app/controllers/script.sl"), blob)
        .map_err(|e| format!("eui_window: cannot write the definitions: {e}"))?;
    // `script#name` falls back to the global function `name` once no
    // controller of that name exists (`serve::eui::resolve`).
    let routes = format!("router_eui(\"{name}\", \"script#{handler}\", \"script#{view}\")\n");
    std::fs::write(root.join("config/routes.sl"), routes)
        .map_err(|e| format!("eui_window: cannot write the routes: {e}"))?;
    Ok(dir)
}

/// Serve `app` on a background thread, on loopback and a port the OS picks,
/// with the loopback gate armed. Returns the port and the session the window
/// presents.
fn serve(app: &std::path::Path) -> Result<(u16, String), String> {
    // Loopback only, whatever the environment says: the window is local.
    std::env::set_var("SOLI_HOST", "127.0.0.1");
    // One publisher key per user, not one per run in a temporary directory —
    // which would also announce itself on every start.
    if std::env::var_os("SOLI_EUI_KEY").is_none() {
        if let Some(dir) = dirs::state_dir().or_else(dirs::data_local_dir) {
            std::env::set_var("SOLI_EUI_KEY", dir.join("soli").join("eui_script.pkcs8"));
        }
    }
    crate::serve::set_quiet(true);

    let (tx, rx) = std::sync::mpsc::channel::<(u16, String)>();
    let folder = app.to_path_buf();
    std::thread::Builder::new()
        .name("soli-eui-server".to_string())
        .spawn(move || {
            let hook: crate::serve::BoundPortHook = Box::new(move |bound| {
                let session = crate::desktop::token::arm_session();
                let _ = tx.send((bound, session));
            });
            if let Err(e) =
                crate::serve::serve_folder_with_options_and_hooks(&folder, 0, false, 2, Some(hook))
            {
                eprintln!("eui_window: the server stopped: {e}");
            }
        })
        .map_err(|e| format!("eui_window: cannot start the server: {e}"))?;
    rx.recv()
        .map_err(|_| "eui_window: the server stopped before it was listening".to_string())
}

#[cfg(feature = "eui-desktop")]
fn window(url: String, cookie: String, title: &str) -> Result<(), String> {
    eui_client::app::launch(eui_client::app::Launch {
        url,
        // The script is the person's own code, run by them.
        allowed: eui_proto::caps::ALL,
        title: title.to_string(),
        cookie: Some(cookie),
        host_loopback: true,
    })
}

/// Unreachable: `open` refuses before serving when there is no window.
#[cfg(not(feature = "eui-desktop"))]
fn window(_url: String, _cookie: String, _title: &str) -> Result<(), String> {
    Err("eui_window: this soli has no built-in EUI window".to_string())
}
