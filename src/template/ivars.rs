//! The `@name` reads in a view's `<% … %>` code — shared by the LSP's
//! undefined-ivar warning (`lsp::view_ivars`) and the code graph's
//! action → view `assigns` edges (`graph::builder`).

use std::collections::HashSet;

/// One `@name` read in a view's code: the name and its byte range in the
/// view's text (the `@` included).
pub(crate) struct IvarRead {
    pub name: String,
    pub start: usize,
    pub end: usize,
}

/// The `@name` reads inside the `<% … %>` tags of `text` (not in `<%# %>`
/// comments, not in string literals, not `email@host`), and the names the
/// view assigns itself (`<% @title = "x" %>`), which it may then read.
pub(crate) fn ivar_reads(text: &str) -> (Vec<IvarRead>, HashSet<String>) {
    let bytes = text.as_bytes();
    let mut reads = Vec::new();
    let mut assigned = HashSet::new();
    let mut i = 0;
    while let Some(open) = text[i..].find("<%") {
        let start = i + open + 2;
        let Some(close) = text[start..].find("%>") else {
            break;
        };
        let end = start + close;
        i = end + 2;
        if bytes.get(start) == Some(&b'#') {
            continue;
        }
        let mut quote: Option<u8> = None;
        let mut j = start;
        while j < end {
            let b = bytes[j];
            if let Some(q) = quote {
                if b == b'\\' {
                    j += 2;
                    continue;
                }
                if b == q {
                    quote = None;
                }
                j += 1;
                continue;
            }
            match b {
                b'"' | b'\'' => quote = Some(b),
                b'#' => break, // a comment to the end of the tag
                b'@' => {
                    let before = if j > 0 { bytes[j - 1] } else { b' ' };
                    let starts_name = bytes
                        .get(j + 1)
                        .is_some_and(|c| c.is_ascii_lowercase() || *c == b'_');
                    if starts_name
                        && !(before.is_ascii_alphanumeric()
                            || before == b'_'
                            || before == b'@'
                            || before == b'.')
                    {
                        let mut k = j + 1;
                        while k < end && (bytes[k].is_ascii_alphanumeric() || bytes[k] == b'_') {
                            k += 1;
                        }
                        // `?` / `!` end a method name: `@ready?` calls one.
                        let method = matches!(bytes.get(k), Some(b'?' | b'!' | b'('));
                        let name = text[j + 1..k].to_string();
                        let rest = text[k..end].trim_start();
                        let assigns = (rest.starts_with('=') && !rest.starts_with("=="))
                            || ["||=", "&&=", "??=", "+=", "-=", "<<"]
                                .iter()
                                .any(|op| rest.starts_with(op));
                        if assigns {
                            assigned.insert(name);
                        } else if !method {
                            reads.push(IvarRead {
                                name,
                                start: j,
                                end: k,
                            });
                        }
                        j = k;
                        continue;
                    }
                }
                _ => {}
            }
            j += 1;
        }
    }
    (reads, assigned)
}
