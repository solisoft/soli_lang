//! Streaming from Soli: `Xml.each(path, match, ns = nil) do |node| … end`
//! and `Xml.reader(path, match, ns = nil)` (an `XmlReader`), over the core's
//! [`crate::xml::Pull`].
//!
//! Both read **files only**, through the same jail as `File` and
//! `Xml.parse_file` (`file::resolve_readable_path`, then `open_for_read`
//! with `O_NOFOLLOW`): a request body can never become a filesystem read,
//! and under `soli serve` and jobs a path outside the app root is refused.
//!
//! **`Xml.each`** needs its block run once per match, which a native cannot
//! do: the engines intercept it by name with the block builder's sites
//! (`block::takes_block`) and [`each`] drives the stream, calling the block
//! with each match. No tree or reader borrow is held while the block runs.
//! The deadline is checked before each block call and every 256 events
//! while scanning; an error raised by the block propagates; the file is
//! closed when the driver returns, on every path. `break` in the block acts
//! as `next` (the native-iterator rule): use `Xml.reader` to stop early.
//!
//! **`XmlReader`** is a pull cursor: `next` (run when read bare) returns the
//! next match or `nil` at the end, `close` closes the file (dropping the
//! reader does too), `count` is the number of matches returned so far.
//!
//! Each match is a detached `XmlNode` in a tree of its own (`parent` and
//! `document` are `nil`) carrying the namespaces in scope where it stood.

use std::cell::RefCell;
use std::fs::File;
use std::io::BufReader;
use std::rc::Rc;

use super::value::{add_static, wrap, Arity, MethodTable, XmlPayload, XmlRef, HANDLE_FIELD};
use crate::error::RuntimeError;
use crate::interpreter::value::{
    Class, HashKey, Instance, NativeFunction, Value, NATIVE_CONSTRUCTOR,
};
use crate::span::Span;
use crate::xml::stream::INTERRUPTED;
use crate::xml::{Matcher, ParsePolicy, Pull, XmlError, XmlErrorKind};

/// What an `XmlReader` holds: the open stream (`None` once closed or at the
/// end), the path as given, and the matches returned so far.
pub struct StreamState {
    pull: Option<Pull<BufReader<File>>>,
    path: String,
    count: usize,
}

// --------------------------------------------------------------- opening

fn is_callable(value: &Value) -> bool {
    matches!(
        value,
        Value::Function(_) | Value::NativeFunction(_) | Value::VmClosure(_)
    )
}

fn usage(who: &str) -> String {
    format!("{who}(path, match, ns = nil)")
}

/// The `ns` table: `nil`, or a Hash of prefix => URI (`""` = the namespace
/// an unprefixed name must be in).
fn ns_table(value: Option<&Value>, who: &str) -> Result<Vec<(String, String)>, String> {
    let hash = match value {
        None | Some(Value::Null) => return Ok(Vec::new()),
        Some(Value::Hash(hash)) => hash,
        Some(other) => {
            return Err(format!(
                "{who}: the namespace table must be a Hash of prefix => URI, got {}",
                other.type_name()
            ))
        }
    };
    let hash = hash.borrow();
    let mut out = Vec::with_capacity(hash.len());
    for (key, value) in hash.iter() {
        let prefix = match key {
            HashKey::String(s) | HashKey::Symbol(s) => s.to_string(),
            _ => return Err(format!("{who}: namespace prefixes must be Strings")),
        };
        let Value::String(uri) = value else {
            return Err(format!(
                "{who}: the URI of prefix '{prefix}' must be a String, got {}",
                value.type_name()
            ));
        };
        out.push((prefix, uri.to_string()));
    }
    Ok(out)
}

/// Open `args = [path, match, ns?]` as a stream, through the file jail.
fn open(args: &[Value], who: &str) -> Result<StreamState, String> {
    if args.len() < 2 || args.len() > 3 {
        return Err(format!(
            "{} takes 2 or 3 arguments, got {}",
            usage(who),
            args.len()
        ));
    }
    let Value::String(path) = &args[0] else {
        return Err(format!(
            "{who}: the path must be a String, got {} (to stream a String, use Xml.parse(s).find_all(…))",
            args[0].type_name()
        ));
    };
    let Value::String(expr) = &args[1] else {
        return Err(format!(
            "{who}: the match must be a String, got {}",
            args[1].type_name()
        ));
    };
    let table = ns_table(args.get(2), who)?;
    let matcher = Matcher::compile(expr, &table).map_err(|e| format!("{who}: {e}"))?;
    let resolved = crate::interpreter::builtins::file::resolve_readable_path(path, who)?;
    // Refuse a FIFO or a device before opening it: opening a FIFO for
    // reading blocks until a writer comes. The check after the open below
    // is the one that counts (the path could change in between).
    if let Ok(meta) = std::fs::symlink_metadata(&resolved) {
        if !meta.is_file() && !meta.file_type().is_symlink() {
            return Err(format!("{who}: cannot read {path}: not a file"));
        }
    }
    let file = crate::interpreter::builtins::file::open_for_read(&resolved, false)
        .map_err(|e| format!("{who}: cannot open {path}: {e}"))?;
    let is_file = file
        .metadata()
        .map_err(|e| format!("{who}: cannot read {path}: {e}"))?
        .is_file();
    if !is_file {
        return Err(format!("{who}: cannot read {path}: not a file"));
    }
    Ok(StreamState {
        pull: Some(Pull::new(BufReader::new(file), matcher, ParsePolicy::xml())),
        path: path.to_string(),
        count: 0,
    })
}

/// The next match of `state`, as a node handle; `None` at the end. Errors
/// close the stream and carry their line and column.
fn next_match(state: &mut StreamState, who: &str) -> Result<Option<Value>, String> {
    let Some(pull) = state.pull.as_mut() else {
        return Ok(None);
    };
    // `exhausted` reads the clock on each call; the stream calls it every
    // 256 events. (`expired` is itself throttled to one clock read every
    // few thousand calls, which would let a scan run far past the budget.)
    match pull.next_checked(&mut crate::interpreter::deadline::exhausted) {
        Ok(Some((tree, id))) => {
            state.count += 1;
            Ok(Some(wrap(XmlRef::new(tree, id))))
        }
        Ok(None) => {
            state.pull = None;
            Ok(None)
        }
        Err(err) => {
            let message = stream_error(pull, err, who);
            state.pull = None;
            Err(message)
        }
    }
}

fn stream_error(pull: &mut Pull<BufReader<File>>, err: XmlError, who: &str) -> String {
    if matches!(&err.kind, XmlErrorKind::Syntax(m) if m == INTERRUPTED) {
        return crate::interpreter::deadline::timeout_message();
    }
    format!("{who}: {}", pull.locate(err))
}

// ------------------------------------------------------------------ each

/// `Xml.each(path, match, ns = nil) do |node| … end`, run by the engines:
/// `args` without the block, `invoke(block, node)` calls the block. Returns
/// the number of matches yielded.
pub fn each(
    args: Vec<Value>,
    block: Value,
    mut invoke: impl FnMut(Value, Value) -> Result<Value, RuntimeError>,
    span: Span,
) -> Result<Value, RuntimeError> {
    const WHO: &str = "Xml.each";
    let general = |message: String| RuntimeError::General { message, span };
    let mut state = open(&args, WHO).map_err(general)?;
    while let Some(node) = next_match(&mut state, WHO).map_err(general)? {
        if crate::interpreter::deadline::expired() {
            return Err(crate::interpreter::deadline::timeout_error(span));
        }
        invoke(block.clone(), node)?;
    }
    Ok(Value::Int(i64::try_from(state.count).unwrap_or(i64::MAX)))
}

// ---------------------------------------------------------------- statics

/// `Xml.each` and `Xml.reader`.
pub(crate) fn xml_statics(table: &mut MethodTable) {
    // Reached only when no engine site ran the block (there is none, or a
    // call path misses the interception).
    add_static(table, "Xml", "each", Arity::Variadic, |args| {
        if args.iter().any(is_callable) {
            return Err(
                "Xml.each: the block could not be run here (an engine call site misses the XML block interception)"
                    .to_string(),
            );
        }
        Err("Xml.each expects a block: Xml.each(path, \"item\") do |node| … end".to_string())
    });
    add_static(table, "Xml", "reader", Arity::Variadic, |args| {
        if args.iter().any(is_callable) {
            return Err(
                "Xml.reader takes no block: use Xml.each(path, match) do |node| … end".to_string(),
            );
        }
        let state = open(args, "Xml.reader")?;
        Ok(wrap_reader(state))
    });
}

// ------------------------------------------------------------ the class

const CLASS_NAME: &str = "XmlReader";

fn reader_of(args: &[Value], method: &str) -> Result<Rc<XmlPayload>, String> {
    if let Some(Value::Instance(inst)) = args.first() {
        let inst = inst.borrow();
        if is_reader_class(&inst.class) {
            if let Some(Value::Xml(payload)) = inst.fields.get(HANDLE_FIELD) {
                if matches!(&**payload, XmlPayload::Reader(_)) {
                    return Ok(payload.clone());
                }
            }
        }
    }
    Err(format!("XmlReader.{method} must be called on an XmlReader"))
}

fn with_state<T>(
    args: &[Value],
    method: &str,
    f: impl FnOnce(&mut StreamState) -> Result<T, String>,
) -> Result<T, String> {
    let payload = reader_of(args, method)?;
    let XmlPayload::Reader(state) = &*payload else {
        return Err(format!("XmlReader.{method} must be called on an XmlReader"));
    };
    let mut state = state
        .try_borrow_mut()
        .map_err(|_| "the XmlReader is already in use".to_string())?;
    f(&mut state)
}

fn add_reader_method(
    table: &mut MethodTable,
    name: &str,
    f: impl Fn(&[Value]) -> Result<Value, String> + 'static,
) {
    // Every reader method takes no argument and runs when read bare.
    table.insert(
        name.to_string(),
        Rc::new(NativeFunction::new_with_auto(
            format!("{CLASS_NAME}.{name}"),
            Some(0),
            true,
            f,
        )),
    );
}

fn reader_methods() -> MethodTable {
    let mut t = MethodTable::new();
    add_reader_method(&mut t, "next", |args| {
        with_state(args, "next", |state| {
            Ok(next_match(state, "Xml.reader")?.unwrap_or(Value::Null))
        })
    });
    add_reader_method(&mut t, "close", |args| {
        with_state(args, "close", |state| {
            state.pull = None;
            Ok(Value::Null)
        })
    });
    add_reader_method(&mut t, "count", |args| {
        with_state(args, "count", |state| {
            Ok(Value::Int(i64::try_from(state.count).unwrap_or(i64::MAX)))
        })
    });
    t
}

fn build_reader_class() -> Rc<Class> {
    let refusal = format!("{CLASS_NAME} has no constructor: use Xml.reader(path, match)");
    let mut methods = reader_methods();
    let constructor_refusal = refusal.clone();
    methods.insert(
        NATIVE_CONSTRUCTOR.to_string(),
        Rc::new(NativeFunction::new(
            CLASS_NAME.to_string(),
            None,
            move |_| Err(constructor_refusal.clone()),
        )),
    );
    let mut statics = MethodTable::new();
    add_static(
        &mut statics,
        CLASS_NAME,
        "new",
        Arity::Optional,
        move |_| Err(refusal.clone()),
    );
    let mut const_fields = std::collections::HashSet::new();
    const_fields.insert(HANDLE_FIELD.to_string());
    Rc::new(Class {
        name: CLASS_NAME.to_string(),
        native_methods: methods,
        native_static_methods: statics,
        const_fields,
        ..Default::default()
    })
}

thread_local! {
    static READER_CLASS: Rc<Class> = build_reader_class();
}

/// The `XmlReader` class of this thread.
pub fn reader_class() -> Rc<Class> {
    READER_CLASS.with(Rc::clone)
}

/// Is `class` this thread's `XmlReader`? A name test, then a pointer
/// comparison.
#[inline]
pub fn is_reader_class(class: &Rc<Class>) -> bool {
    class.name == CLASS_NAME && READER_CLASS.with(|c| Rc::ptr_eq(c, class))
}

fn wrap_reader(state: StreamState) -> Value {
    let mut inst = Instance::new(reader_class());
    inst.set(
        HANDLE_FIELD,
        Value::Xml(Rc::new(XmlPayload::Reader(RefCell::new(state)))),
    );
    Value::Instance(Rc::new(RefCell::new(inst)))
}

/// `print(reader)`, `"#{reader}"`: `<XmlReader exports/big.xml>`.
pub fn display(inst: &Instance) -> Option<String> {
    if !is_reader_class(&inst.class) {
        return None;
    }
    let path = match inst.fields.get(HANDLE_FIELD) {
        Some(Value::Xml(payload)) => match &**payload {
            XmlPayload::Reader(state) => state
                .try_borrow()
                .map(|s| s.path.clone())
                .unwrap_or_default(),
            XmlPayload::Node(_) => String::new(),
        },
        _ => String::new(),
    };
    Some(format!("<{CLASS_NAME} {path}>"))
}
