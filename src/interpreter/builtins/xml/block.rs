//! The block builder: `Xml.build(name, attrs) do |root| … end` and
//! `node.add(name, content, attrs) do |child| … end`; and streaming's
//! `Xml.each(path, match) do |node| … end`, whose block runs once per match
//! (`reader::each`).
//!
//! A native cannot call Soli code, so the engines intercept these natives
//! by name at every site where a native is invoked with its arguments
//! (tree-walker: `call_value`, `call_value_with_named` and the direct
//! instance-native path; VM: `call_native` and the direct instance-native
//! path of `call_method_slow_path`) and hand them to [`run`], with a closure
//! that calls a Soli value. Without a block the call is the plain native
//! call.
//!
//! The block runs **after** the native returned, so no tree borrow is held
//! while Soli code runs. The deadline is checked before the block is called;
//! an error raised by the block propagates; the block's value is ignored and
//! the call returns the document (`Xml.build`) or the new child (`add`).
//! `break` inside an `Xml.each` block stops the walk, which answers nil, as
//! every iterator's does (#651); inside a builder block it leaves the block,
//! and the call still answers what it built.

use crate::error::RuntimeError;
use crate::interpreter::value::Value;
use crate::span::Span;

/// Is `name` a native whose trailing block the engines must run? Bound
/// methods are named after the receiver's class (`XmlDocument.add`), the
/// class table's own entry after `XmlNode`.
#[inline]
pub fn takes_block(name: &str) -> bool {
    name.starts_with("Xml")
        && matches!(
            name,
            "Xml.build" | "Xml.each" | "XmlNode.add" | "XmlDocument.add"
        )
}

fn is_callable(value: &Value) -> bool {
    matches!(
        value,
        Value::Function(_) | Value::NativeFunction(_) | Value::VmClosure(_)
    )
}

/// Take the block out of a native call's arguments: the last argument, or
/// the one just before a trailing Hash. The tree-walker passes
/// `[…, named_hash, block]` and the VM `[…, block, named_hash]` for
/// `add("item", id: 1) do … end`; both come out as `[…, named_hash]`.
pub fn split_block(args: &mut Vec<Value>) -> Option<Value> {
    let n = args.len();
    if n > 0 && is_callable(&args[n - 1]) {
        return args.pop();
    }
    if n > 1 && matches!(args[n - 1], Value::Hash(_)) && is_callable(&args[n - 2]) {
        return Some(args.remove(n - 2));
    }
    None
}

/// What the block receives: the root element for `Xml.build`, the new child
/// for `add`.
fn block_argument(name: &str, result: &Value) -> Result<Value, String> {
    if name != "Xml.build" {
        return Ok(result.clone());
    }
    let Some(doc) = super::value::value_as_xml(result) else {
        return Err("Xml.build: did not build a document".to_string());
    };
    let root = doc.read(|tree, _| tree.root_element())?;
    root.map(|id| super::value::wrap(doc.at(id)))
        .ok_or_else(|| "Xml.build: the document has no root element".to_string())
}

/// Run a block-taking XML native. `call` runs the native on the arguments
/// (block removed); `invoke` calls the block with one argument. `Xml.each`
/// calls it once per match; without a block, its native raises.
pub fn run(
    name: &str,
    mut args: Vec<Value>,
    call: impl FnOnce(&[Value]) -> Result<Value, String>,
    mut invoke: impl FnMut(Value, Value) -> Result<Value, RuntimeError>,
    span: Span,
) -> Result<Value, RuntimeError> {
    let block = split_block(&mut args);
    if name == "Xml.each" {
        if let Some(block) = block {
            // `break` in the block stops the walk, which answers nil, as
            // every iterator does (#651).
            return crate::interpreter::executor::stop_on_block_break(super::reader::each(
                args, block, invoke, span,
            ));
        }
    }
    let result = call(&args).map_err(|message| RuntimeError::General { message, span })?;
    let Some(block) = block else {
        return Ok(result);
    };
    let subject =
        block_argument(name, &result).map_err(|message| RuntimeError::General { message, span })?;
    if crate::interpreter::deadline::expired() {
        return Err(crate::interpreter::deadline::timeout_error(span));
    }
    // `break` in a builder block leaves that block; the call still answers
    // what it built.
    match invoke(block, subject) {
        Ok(_) | Err(RuntimeError::BlockBreak(_)) => Ok(result),
        Err(error) => Err(error),
    }
}
