//! LiveView diff engine.
//!
//! Produces positional line-splice patches against the previous render
//! instead of sending the entire HTML document on every tick. Most
//! interactive updates only touch a tiny region (a counter value, one bit
//! in the clock, one table row, ...), so shipping just the changed lines
//! is the concrete bandwidth win.
//!
//! Protocol contract with the client (`src/live/client.js`):
//! - The client keeps a *shadow copy* of the exact HTML string it last
//!   received. A `splice` patch is applied positionally to that string:
//!   replace `del` lines starting at line `at` with the lines in `ins`.
//!   The client then morphs the live DOM to match the patched shadow.
//! - Line indexing MUST agree byte-for-byte on both sides, so both split
//!   on `'\n'` (never `str::lines()`, which drops trailing-newline info
//!   and strips `\r`): `lines.join("\n") == original` must always hold.
//! - `replace` carries a whole new document (tiny docs, or when nothing
//!   is common between renders) and resets the client's shadow.

use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Patch {
    /// Replace `del` lines at line index `at` with the lines in `ins`,
    /// applied to the client's shadow copy of the previous render.
    Splice {
        at: usize,
        del: usize,
        ins: Vec<String>,
    },
    /// Full-document replacement; resets the client's shadow.
    Replace { new: String },
}

/// Compute a patch list from old → new HTML.
/// Returns a JSON array of Patch objects (`"[]"` when nothing changed).
pub fn compute_patch(old_html: &str, new_html: &str) -> String {
    if old_html == new_html {
        return "[]".to_string();
    }

    // For tiny documents the overhead of diffing isn't worth it.
    if old_html.len() < 256 || new_html.len() < 256 {
        return compute_full_replace(new_html);
    }

    let old_lines: Vec<&str> = old_html.split('\n').collect();
    let new_lines: Vec<&str> = new_html.split('\n').collect();

    let prefix = common_prefix_len(&old_lines, &new_lines);
    let suffix = common_suffix_len(&old_lines, &new_lines, prefix);

    // No common line at either end: a splice would carry the whole
    // document anyway, so send a plain replace (also resets the shadow).
    if prefix == 0 && suffix == 0 {
        return compute_full_replace(new_html);
    }

    let old_len = old_lines.len();
    let new_len = new_lines.len();

    let del = old_len - prefix - suffix;
    let ins: Vec<String> = new_lines[prefix..new_len - suffix]
        .iter()
        .map(|s| s.to_string())
        .collect();

    if del == 0 && ins.is_empty() {
        return "[]".to_string();
    }

    let patches = vec![Patch::Splice {
        at: prefix,
        del,
        ins,
    }];

    serde_json::to_string(&patches).unwrap_or_else(|_| compute_full_replace(new_html))
}

fn common_prefix_len(a: &[&str], b: &[&str]) -> usize {
    a.iter().zip(b.iter()).take_while(|(x, y)| x == y).count()
}

fn common_suffix_len(a: &[&str], b: &[&str], prefix: usize) -> usize {
    let mut i = 0usize;
    let max = (a.len() - prefix).min(b.len() - prefix);
    while i < max {
        if a[a.len() - 1 - i] != b[b.len() - 1 - i] {
            break;
        }
        i += 1;
    }
    i
}

fn compute_full_replace(html: &str) -> String {
    let patches = vec![Patch::Replace {
        new: html.to_string(),
    }];

    serde_json::to_string(&patches).expect("serializing a string-only patch cannot fail")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Mirror of the client's `spliceLines` (src/live/client.js): the
    /// round-trip property `apply(old, patch) == new` pins the byte-exact
    /// shadow contract between server and client.
    fn apply_patches(old: &str, patch_json: &str) -> String {
        #[derive(serde::Deserialize)]
        #[serde(tag = "type", rename_all = "lowercase")]
        enum WirePatch {
            Splice {
                at: usize,
                del: usize,
                ins: Vec<String>,
            },
            Replace {
                new: String,
            },
        }

        let patches: Vec<WirePatch> = serde_json::from_str(patch_json).expect("valid patch JSON");
        let mut shadow = old.to_string();
        for patch in patches {
            match patch {
                WirePatch::Splice { at, del, ins } => {
                    let mut lines: Vec<String> =
                        shadow.split('\n').map(|s| s.to_string()).collect();
                    assert!(at + del <= lines.len(), "splice out of bounds");
                    lines.splice(at..at + del, ins);
                    shadow = lines.join("\n");
                }
                WirePatch::Replace { new } => shadow = new,
            }
        }
        shadow
    }

    /// A multi-line document comfortably above the 256-byte full-replace
    /// threshold, with `marker` on one middle line.
    fn big_doc(marker: &str) -> String {
        let mut lines: Vec<String> = (0..10)
            .map(|i| format!("<div class=\"row-padding-{i}\">static line {i}</div>"))
            .collect();
        lines.insert(5, format!("<span id=\"c\">{marker}</span>"));
        lines.join("\n")
    }

    #[test]
    fn identical_yields_empty_patch() {
        let html = big_doc("42");
        assert_eq!(compute_patch(&html, &html), "[]");
    }

    #[test]
    fn tiny_doc_yields_full_replace() {
        let old = "<div id=\"c\">42</div>";
        let new = "<div id=\"c\">43</div>";
        let json = compute_patch(old, new);
        assert!(json.contains("\"type\":\"replace\""));
        assert!(json.contains("43"));
        assert_eq!(apply_patches(old, &json), new);
    }

    #[test]
    fn middle_line_change_yields_single_splice() {
        let old = big_doc("42");
        let new = big_doc("43");
        let json = compute_patch(&old, &new);
        assert_eq!(
            json,
            r#"[{"type":"splice","at":5,"del":1,"ins":["<span id=\"c\">43</span>"]}]"#
        );
        assert_eq!(apply_patches(&old, &json), new);
    }

    #[test]
    fn pure_insertion_has_zero_del() {
        let old = big_doc("42");
        let new = {
            let mut lines: Vec<&str> = old.split('\n').collect();
            lines.insert(5, "<p>inserted</p>");
            lines.join("\n")
        };
        let json = compute_patch(&old, &new);
        assert!(json.contains("\"del\":0"), "got: {json}");
        assert_eq!(apply_patches(&old, &json), new);
    }

    #[test]
    fn pure_deletion_has_empty_ins() {
        let old = big_doc("42");
        let new = {
            let mut lines: Vec<&str> = old.split('\n').collect();
            lines.remove(5);
            lines.join("\n")
        };
        let json = compute_patch(&old, &new);
        assert!(json.contains("\"ins\":[]"), "got: {json}");
        assert_eq!(apply_patches(&old, &json), new);
    }

    #[test]
    fn line_becoming_empty_is_not_a_deletion() {
        // Replacing a line with an empty line must keep the line count:
        // `ins:[""]`, never `ins:[]`.
        let old = big_doc("42");
        let new = {
            let mut lines: Vec<&str> = old.split('\n').collect();
            lines[5] = "";
            lines.join("\n")
        };
        let json = compute_patch(&old, &new);
        assert!(json.contains("\"ins\":[\"\"]"), "got: {json}");
        assert_eq!(apply_patches(&old, &json), new);
    }

    #[test]
    fn first_line_change_splices_at_zero() {
        let old = big_doc("42");
        let new = {
            let mut lines: Vec<&str> = old.split('\n').collect();
            lines[0] = "<div>changed head</div>";
            lines.join("\n")
        };
        let json = compute_patch(&old, &new);
        assert!(json.contains("\"at\":0"), "got: {json}");
        assert_eq!(apply_patches(&old, &json), new);
    }

    #[test]
    fn last_line_change_round_trips() {
        let old = big_doc("42");
        let new = {
            let mut lines: Vec<&str> = old.split('\n').collect();
            let last = lines.len() - 1;
            lines[last] = "<div>changed tail</div>";
            lines.join("\n")
        };
        let json = compute_patch(&old, &new);
        assert_eq!(apply_patches(&old, &json), new);
    }

    #[test]
    fn repeated_identical_lines_round_trip() {
        // Prefix/suffix overlap stress: identical filler lines surround the
        // change, so prefix + suffix must not double-count the middle. Both
        // documents are above the 256-byte full-replace threshold.
        let filler = "<li class=\"px-4 py-2 border-b border-stone-200\">item</li>";
        let mut old_lines = vec!["<ul class=\"divide-y\">"];
        old_lines.extend(std::iter::repeat_n(filler, 8));
        old_lines.push("</ul>");
        let mut new_lines = vec!["<ul class=\"divide-y\">"];
        new_lines.extend(std::iter::repeat_n(filler, 7));
        new_lines.push("</ul>");

        let old = old_lines.join("\n");
        let new = new_lines.join("\n");
        assert!(old.len() >= 256 && new.len() >= 256);

        let json = compute_patch(&old, &new);
        assert!(json.contains("\"type\":\"splice\""), "got: {json}");
        assert_eq!(apply_patches(&old, &json), new);
    }

    #[test]
    fn trailing_newline_round_trips() {
        // `.lines()` would eat the trailing newline; `split('\n')` must not.
        let old = format!("{}\n", big_doc("42"));
        let new = format!("{}\n", big_doc("43"));
        let json = compute_patch(&old, &new);
        assert_eq!(apply_patches(&old, &json), new);
    }

    #[test]
    fn crlf_round_trips() {
        let old = big_doc("42").replace('\n', "\r\n");
        let new = big_doc("43").replace('\n', "\r\n");
        let json = compute_patch(&old, &new);
        assert_eq!(apply_patches(&old, &json), new);
    }

    #[test]
    fn no_common_lines_yields_full_replace() {
        let old = big_doc("42");
        let new: String = (0..12)
            .map(|i| format!("<section>totally different content block {i}</section>"))
            .collect::<Vec<_>>()
            .join("\n");
        let json = compute_patch(&old, &new);
        assert!(json.contains("\"type\":\"replace\""), "got: {json}");
        assert_eq!(apply_patches(&old, &json), new);
    }
}
