//! JSON output for Soli values, written straight into a byte buffer.
//!
//! This is the response path of every `render_json` and `JSON.stringify`, so it
//! does not go through serde: a `Serializer` dispatches each value through a
//! generic trait and a map-key serializer, where here a value is one `match`
//! and a `Vec::extend_from_slice`. The output is the one sonic-rs gave before
//! this — serde_json's escapes, ryu for floats, `null` for a non-finite
//! float — and it follows `Value`'s own
//! `Serialize` impl in everything else: the depth limit, `DateTime` as RFC 3339,
//! enum tags, and the sensitive-field filter on instances.

use crate::interpreter::value::{
    enum_variant_tag, is_safe_serialised_field, HashKey, HashPairs, Value,
};

/// Maximum container nesting, as in `Value`'s `Serialize` impl.
const MAX_DEPTH: usize = 512;

/// Serialize a Value to a JSON string.
#[inline]
pub fn stringify_to_string(value: &Value) -> Result<String, String> {
    with_scratch(|out| write_value(out, value, 0))
}

/// Serialize an array slice to JSON without cloning into a Value.
#[inline]
pub fn stringify_array_to_string(items: &[Value]) -> Result<String, String> {
    with_scratch(|out| write_array(out, items, 0))
}

/// Serialize hash entries to JSON without cloning into a Value. Only string
/// keys are written.
#[inline]
pub fn stringify_hash_entries_to_string(entries: &[(HashKey, Value)]) -> Result<String, String> {
    with_scratch(|out| write_string_keyed(out, entries.iter().map(|(k, v)| (k, v)), 0))
}

/// Serialize a borrowed hash map to JSON without cloning its entries into a
/// Vec. Only string keys are written.
#[inline]
pub fn stringify_hash_map_to_string(map: &HashPairs) -> Result<String, String> {
    with_scratch(|out| write_string_keyed(out, map.iter(), 0))
}

/// A scratch buffer kept above this is given back after use, so one huge
/// document does not pin its size on the thread for good.
const SCRATCH_KEEP: usize = 1 << 20;

thread_local! {
    static SCRATCH: std::cell::Cell<Vec<u8>> = const { std::cell::Cell::new(Vec::new()) };
}

/// Write into the thread's scratch buffer, which keeps its capacity from one
/// call to the next, then copy the result out at its exact size: one
/// allocation per document instead of a doubling chain from a few bytes.
/// (Measured: allocating a buffer sized from the previous document and
/// returning it uncopied was slower — a fresh large allocation costs more
/// than copying out of a warm one.) The buffer is taken out of the cell for
/// the duration, so a nested call (a deferred read resolved mid-write) simply
/// starts from an empty one.
fn with_scratch(write: impl FnOnce(&mut Vec<u8>) -> Result<(), String>) -> Result<String, String> {
    let mut out = SCRATCH.with(std::cell::Cell::take);
    out.clear();
    let result = write(&mut out);
    // SAFETY: every byte written is ASCII punctuation, ASCII digits, an escape
    // sequence, or a whole run copied from a `&str` — so the buffer is UTF-8.
    let text = result.map(|()| unsafe { String::from_utf8_unchecked(out.as_slice().to_vec()) });
    if out.capacity() > SCRATCH_KEEP {
        out = Vec::new();
    }
    SCRATCH.with(|cell| cell.set(out));
    text
}

fn write_value(out: &mut Vec<u8>, value: &Value, depth: usize) -> Result<(), String> {
    match value {
        Value::Null => out.extend_from_slice(b"null"),
        Value::Bool(true) => out.extend_from_slice(b"true"),
        Value::Bool(false) => out.extend_from_slice(b"false"),
        Value::Int(n) => write_raw(out, itoa::Buffer::new().format(*n).as_bytes()),
        Value::Float(f) => write_float(out, *f),
        Value::String(s) | Value::Symbol(s) => write_str(out, s),
        Value::Decimal(d) => write_str(out, &d.to_string()),
        Value::DateTime(ts, _) => write_str(out, &Value::datetime_to_rfc3339(*ts)),
        Value::Array(items) => write_array(out, &items.borrow(), depth)?,
        Value::Hash(hash) => {
            let depth = enter(depth)?;
            let hash = hash.borrow();
            let open_at = out.len();
            out.push(b'{');
            for (key, item) in hash.iter() {
                let (HashKey::String(key) | HashKey::Symbol(key)) = key else {
                    continue;
                };
                write_key(out, key);
                write_value(out, item, depth)?;
                out.push(b',');
            }
            close(out, open_at, b'}');
        }
        Value::Instance(instance) => {
            let depth = enter(depth)?;
            let instance = instance.borrow();
            // Enum value: a bare tag for a unit variant, otherwise
            // `{"variant": tag, ...payload}`.
            if let Some(tag) = enum_variant_tag(&instance) {
                if instance.fields.len() == 1 {
                    write_str(out, tag);
                    return Ok(());
                }
                out.extend_from_slice(b"{\"variant\":");
                write_str(out, tag);
                for (key, item) in instance.fields.iter() {
                    if key.as_str() != "__variant" {
                        out.push(b',');
                        write_str(out, key);
                        out.push(b':');
                        write_value(out, item, depth)?;
                    }
                }
                out.push(b'}');
                return Ok(());
            }
            // SEC-013: skip sensitive and framework-internal fields.
            let open_at = out.len();
            out.push(b'{');
            for (key, item) in instance.fields.iter() {
                if is_safe_serialised_field(key) {
                    write_key(out, key);
                    write_value(out, item, depth)?;
                    out.push(b',');
                }
            }
            close(out, open_at, b'}');
        }
        // Resolve a `grouped {}` deferred read before writing it.
        Value::Deferred(cell) => {
            let depth = enter(depth)?;
            let resolved = crate::interpreter::builtins::model::batch::force(cell)?;
            write_value(out, &resolved, depth)?;
        }
        other => return Err(format!("Cannot convert {} to JSON", other.type_name())),
    }
    Ok(())
}

#[inline]
fn enter(depth: usize) -> Result<usize, String> {
    if depth >= MAX_DEPTH {
        return Err("structure nested too deeply to serialize".to_string());
    }
    Ok(depth + 1)
}

fn write_array(out: &mut Vec<u8>, items: &[Value], depth: usize) -> Result<(), String> {
    let depth = enter(depth)?;
    let open_at = out.len();
    out.push(b'[');
    for item in items {
        write_value(out, item, depth)?;
        out.push(b',');
    }
    close(out, open_at, b']');
    Ok(())
}

/// End a container opened at `open_at`. Each element was followed by a `,`,
/// so the last one's becomes the closing bracket — no "first element?" test
/// per element. An empty container has none, and gets the bracket appended.
#[inline]
fn close(out: &mut Vec<u8>, open_at: usize, bracket: u8) {
    let len = out.len();
    if len > open_at + 1 {
        out[len - 1] = bracket;
    } else {
        out.push(bracket);
    }
}

/// A top-level hash written from borrowed entries: string keys only, the way
/// the serde wrappers these functions replaced wrote it.
fn write_string_keyed<'a>(
    out: &mut Vec<u8>,
    entries: impl Iterator<Item = (&'a HashKey, &'a Value)>,
    depth: usize,
) -> Result<(), String> {
    let open_at = out.len();
    out.push(b'{');
    for (key, item) in entries {
        if let HashKey::String(key) = key {
            write_key(out, key);
            write_value(out, item, depth)?;
            out.push(b',');
        }
    }
    close(out, open_at, b'}');
    Ok(())
}

#[inline]
fn write_float(out: &mut Vec<u8>, value: f64) {
    if value.is_finite() {
        out.extend_from_slice(ryu::Buffer::new().format_finite(value).as_bytes());
    } else {
        out.extend_from_slice(b"null");
    }
}

/// For each byte: 0 when it is written as is, otherwise the character that
/// follows the backslash in its escape (`u` for the `\u00XX` form).
static ESCAPE: [u8; 256] = {
    let mut table = [0u8; 256];
    let mut i = 0;
    while i < 0x20 {
        table[i] = b'u';
        i += 1;
    }
    table[0x08] = b'b';
    table[0x09] = b't';
    table[0x0a] = b'n';
    table[0x0c] = b'f';
    table[0x0d] = b'r';
    table[b'"' as usize] = b'"';
    table[b'\\' as usize] = b'\\';
    table
};

const ONES: u64 = 0x0101_0101_0101_0101;
const HIGH: u64 = 0x8080_8080_8080_8080;

/// Non-zero when one of the eight bytes of `word` needs escaping: a control
/// character, `"` or `\`. (Exact as a yes/no; it does not say which byte.)
#[inline(always)]
fn word_needs_escape(word: u64) -> u64 {
    let below_space = word.wrapping_sub(0x20 * ONES) & !word & HIGH;
    let quote = word ^ (b'"' as u64 * ONES);
    let backslash = word ^ (b'\\' as u64 * ONES);
    let has_quote = quote.wrapping_sub(ONES) & !quote & HIGH;
    let has_backslash = backslash.wrapping_sub(ONES) & !backslash & HIGH;
    below_space | has_quote | has_backslash
}

/// `"text"`, escaped as JSON requires.
#[inline]
fn write_str(out: &mut Vec<u8>, text: &str) {
    write_quoted(out, text.as_bytes(), b"\"");
}

/// `"key":` — the quotes and colon of an object key in one reservation.
#[inline]
fn write_key(out: &mut Vec<u8>, key: &str) {
    write_quoted(out, key.as_bytes(), b"\":");
}

#[inline(always)]
fn write_quoted(out: &mut Vec<u8>, bytes: &[u8], closing: &'static [u8]) {
    if needs_escape(bytes) {
        write_escaped_str(out, bytes);
        out.extend_from_slice(&closing[1..]);
        return;
    }
    out.reserve(bytes.len() + 1 + closing.len());
    // SAFETY: `reserve` made room for the opening quote, the bytes and
    // `closing`.
    unsafe {
        let len = out.len();
        let dst = out.as_mut_ptr().add(len);
        *dst = b'"';
        copy_bytes(bytes.as_ptr(), dst.add(1), bytes.len());
        copy_bytes(closing.as_ptr(), dst.add(1 + bytes.len()), closing.len());
        out.set_len(len + 1 + bytes.len() + closing.len());
    }
}

/// Bytes that need no escaping (a formatted number).
#[inline(always)]
fn write_raw(out: &mut Vec<u8>, bytes: &[u8]) {
    out.reserve(bytes.len());
    // SAFETY: `reserve` made room for the bytes.
    unsafe {
        let len = out.len();
        copy_bytes(bytes.as_ptr(), out.as_mut_ptr().add(len), bytes.len());
        out.set_len(len + bytes.len());
    }
}

/// Whether any byte of `bytes` is a control character, `"` or `\`.
///
/// Keys, ids and names are short, so under 16 bytes the string is read as at
/// most two overlapping words and tested with `word_needs_escape` — no loop.
/// Longer strings take a branch-free fold the compiler turns into vector
/// compares.
#[inline(always)]
fn needs_escape(bytes: &[u8]) -> bool {
    let len = bytes.len();
    if len >= 16 {
        return bytes.iter().fold(false, |found, &b| {
            found | (b < 0x20) | (b == b'"') | (b == b'\\')
        });
    }
    let p = bytes.as_ptr();
    // SAFETY: every read below lies inside `bytes` (each branch is gated on
    // `len` covering the bytes it reads).
    let word = unsafe {
        if len >= 8 {
            let head = (p as *const u64).read_unaligned();
            let tail = (p.add(len - 8) as *const u64).read_unaligned();
            return word_needs_escape(head) | word_needs_escape(tail) != 0;
        } else if len >= 4 {
            let head = u64::from((p as *const u32).read_unaligned());
            let tail = u64::from((p.add(len - 4) as *const u32).read_unaligned());
            head | (tail << 32)
        } else if len > 0 {
            // Three reads cover one to three bytes; the other five lanes are
            // spaces, which never need escaping.
            u64::from(*p)
                | (u64::from(*p.add(len / 2)) << 8)
                | (u64::from(*p.add(len - 1)) << 16)
                | 0x2020_2020_2000_0000
        } else {
            return false;
        }
    };
    word_needs_escape(word) != 0
}

/// Copy `len` bytes with at most two overlapping moves below 16 bytes, where a
/// call to `memcpy` would cost more than the copy.
///
/// # Safety
/// `src` must be readable and `dst` writable for `len` bytes, not overlapping.
#[inline(always)]
unsafe fn copy_bytes(src: *const u8, dst: *mut u8, len: usize) {
    if len >= 16 {
        std::ptr::copy_nonoverlapping(src, dst, len);
    } else if len >= 8 {
        let head = (src as *const u64).read_unaligned();
        let tail = (src.add(len - 8) as *const u64).read_unaligned();
        (dst as *mut u64).write_unaligned(head);
        (dst.add(len - 8) as *mut u64).write_unaligned(tail);
    } else if len >= 4 {
        let head = (src as *const u32).read_unaligned();
        let tail = (src.add(len - 4) as *const u32).read_unaligned();
        (dst as *mut u32).write_unaligned(head);
        (dst.add(len - 4) as *mut u32).write_unaligned(tail);
    } else if len > 0 {
        *dst = *src;
        *dst.add(len / 2) = *src.add(len / 2);
        *dst.add(len - 1) = *src.add(len - 1);
    }
}

#[cold]
fn write_escaped_str(out: &mut Vec<u8>, bytes: &[u8]) {
    out.reserve(bytes.len() + 2);
    out.push(b'"');
    // `start` is the first byte not yet copied. Clean runs are copied whole;
    // the scan skips eight bytes at a time while none of them needs escaping.
    let mut start = 0;
    let mut i = 0;
    while i < bytes.len() {
        if i + 8 <= bytes.len() {
            let mut word = [0u8; 8];
            word.copy_from_slice(&bytes[i..i + 8]);
            let word = u64::from_le_bytes(word);
            if word_needs_escape(word) == 0 {
                i += 8;
                continue;
            }
        }
        let byte = bytes[i];
        let escape = ESCAPE[byte as usize];
        if escape != 0 {
            out.extend_from_slice(&bytes[start..i]);
            write_escape(out, byte, escape);
            start = i + 1;
        }
        i += 1;
    }
    out.extend_from_slice(&bytes[start..]);
    out.push(b'"');
}

#[cold]
fn write_escape(out: &mut Vec<u8>, byte: u8, escape: u8) {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    if escape == b'u' {
        out.extend_from_slice(&[
            b'\\',
            b'u',
            b'0',
            b'0',
            HEX[(byte >> 4) as usize],
            HEX[(byte & 0xf) as usize],
        ]);
    } else {
        out.extend_from_slice(&[b'\\', escape]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interpreter::value::parse_json;

    /// The writer must agree with serde_json on every value both can express.
    fn same_as_serde_json(value: &Value) -> Result<(), String> {
        let ours = stringify_to_string(value)?;
        let theirs = serde_json::to_string(value).map_err(|e| e.to_string())?;
        assert_eq!(ours, theirs);
        Ok(())
    }

    #[test]
    fn strings_escape_exactly_as_serde_json_does() -> Result<(), String> {
        let mut every_byte = String::new();
        for c in 0u8..0x80 {
            every_byte.push(c as char);
        }
        for text in [
            "",
            "plain",
            "exactly8",
            "a \"quoted\" word",
            "back\\slash",
            "tab\there\nnew line\r\u{8}\u{c}",
            "\u{0}\u{1f}\u{7f}",
            "héllo wörld — ✓ 😀",
            "a long run of clean text before an escape at the very end\"",
            "\"at the start of a long run of otherwise clean text",
            &every_byte,
        ] {
            same_as_serde_json(&Value::String(text.into()))?;
        }
        Ok(())
    }

    #[test]
    fn numbers_match_what_soli_always_wrote() -> Result<(), String> {
        for n in [0, -1, 42, i64::MIN, i64::MAX] {
            same_as_serde_json(&Value::Int(n))?;
        }
        // Floats are ryu's shortest form, as sonic-rs wrote them. (serde_json
        // now spells exponents `1e+21`; Soli's output never did.)
        for (f, text) in [
            (0.0, "0.0"),
            (-0.0, "-0.0"),
            (1.0, "1.0"),
            (0.1, "0.1"),
            (1.5e-7, "1.5e-7"),
            (1e21, "1e21"),
            (123456789.125, "123456789.125"),
            (-2.5, "-2.5"),
        ] {
            assert_eq!(stringify_to_string(&Value::Float(f))?, text);
        }
        assert_eq!(stringify_to_string(&Value::Float(f64::NAN))?, "null");
        assert_eq!(stringify_to_string(&Value::Float(f64::INFINITY))?, "null");
        Ok(())
    }

    #[test]
    fn an_escape_is_found_at_every_position_of_every_short_length() -> Result<(), String> {
        for len in 0..40 {
            same_as_serde_json(&Value::String("k".repeat(len).into()))?;
            for at in 0..len {
                for special in ['"', '\\', '\n', '\u{1}'] {
                    let mut text: Vec<char> = "k".repeat(len).chars().collect();
                    text[at] = special;
                    let text: String = text.into_iter().collect();
                    same_as_serde_json(&Value::String(text.as_str().into()))?;
                    // As a key, too: `"key":` takes the same paths.
                    let key = serde_json::to_string(&text).map_err(|e| e.to_string())?;
                    same_as_serde_json(&parse_json(&format!("{{{key}:1}}"))?)?;
                }
            }
        }
        Ok(())
    }

    #[test]
    fn empty_and_single_element_containers_close_correctly() -> Result<(), String> {
        for json in [
            "[]",
            "{}",
            "[1]",
            "{\"a\":1}",
            "[[],{}]",
            "{\"a\":[],\"b\":{}}",
        ] {
            same_as_serde_json(&parse_json(json)?)?;
        }
        Ok(())
    }

    #[test]
    fn nested_documents_match_serde_json() -> Result<(), String> {
        let json = r#"{"users":[{"id":1,"name":"Ann \"A\"","tags":["x","y"],"score":1.5,
            "active":true,"manager":null,"meta":{"deep":{"deeper":[1,[2,[3]]]}}}],"count":1}"#;
        let value = parse_json(json)?;
        same_as_serde_json(&value)?;
        assert_eq!(
            stringify_to_string(&Value::Array(Default::default()))?,
            "[]"
        );
        Ok(())
    }

    #[test]
    fn nesting_past_the_limit_is_an_error() -> Result<(), String> {
        let mut value = Value::Null;
        for _ in 0..MAX_DEPTH + 1 {
            value = Value::Array(std::rc::Rc::new(std::cell::RefCell::new(vec![value])));
        }
        let Err(err) = stringify_to_string(&value) else {
            return Err("a 513-deep array serialized".to_string());
        };
        assert!(err.contains("nested too deeply"), "{err}");
        Ok(())
    }
}
