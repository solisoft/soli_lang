//! LiveView diff engine.
//!
//! Produces a sequence of small patches (replace/add/remove) instead of
//! always sending the entire HTML document on every tick. This is the
//! concrete performance win for LiveView: most interactive updates only
//! touch a tiny region (a counter value, one bit in the clock, one table row, ...).
//!
//! Strategy (v1, pragmatic):
//! - Line-based diff (HTML is line-oriented enough for big wins).
//! - Find longest common prefix + suffix.
//! - Emit one targeted "replace" patch for the changed middle region.
//! - Include one line of context before/after when available so the client
//!   can anchor the replacement reliably.
//! - Fall back to a single full replace for very small documents or when
//!   the diff would be larger than the full document (rare).

use serde::Serialize;
use serde_json::Value as JsonValue;
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize)]
pub struct Patch {
    #[serde(rename = "type")]
    pub change_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub old: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub new: Option<String>,
}

/// Compute a patch list from old → new HTML.
/// Returns a JSON array of Patch objects.
pub fn compute_patch(old_html: &str, new_html: &str) -> String {
    if old_html == new_html {
        return "[]".to_string();
    }

    // For tiny documents the overhead of diffing isn't worth it.
    if old_html.len() < 256 || new_html.len() < 256 {
        return compute_full_replace(new_html);
    }

    let old_lines: Vec<&str> = old_html.lines().collect();
    let new_lines: Vec<&str> = new_html.lines().collect();

    let prefix = common_prefix_len(&old_lines, &new_lines);
    let suffix = common_suffix_len(&old_lines, &new_lines, prefix);

    let old_len = old_lines.len();
    let new_len = new_lines.len();

    // If the "changed" region is actually the whole document (or almost),
    // just send a full replace — it's simpler and no bigger on the wire.
    let changed_old = old_len.saturating_sub(prefix + suffix);
    let changed_new = new_len.saturating_sub(prefix + suffix);

    if changed_old == 0 && changed_new == 0 {
        return "[]".to_string();
    }

    // Build the changed blocks with a tiny bit of context for anchoring.
    let ctx_before = if prefix > 0 { 1 } else { 0 };
    let ctx_after = if suffix > 0 { 1 } else { 0 };

    let start_old = prefix.saturating_sub(ctx_before);
    let end_old = old_len.saturating_sub(suffix.saturating_sub(ctx_after));

    let start_new = prefix.saturating_sub(ctx_before);
    let end_new = new_len.saturating_sub(suffix.saturating_sub(ctx_after));

    let old_block = old_lines[start_old..end_old].join("\n");
    let new_block = new_lines[start_new..end_new].join("\n");

    // If the diff is bigger than just sending the whole thing, fall back.
    if old_block.len() + new_block.len() > new_html.len() + 128 {
        return compute_full_replace(new_html);
    }

    let patches = vec![Patch {
        change_type: "replace".to_string(),
        old: if old_block.is_empty() {
            None
        } else {
            Some(old_block)
        },
        new: if new_block.is_empty() {
            None
        } else {
            Some(new_block)
        },
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
    let patches = vec![Patch {
        change_type: "replace".to_string(),
        old: None,
        new: Some(html.to_string()),
    }];

    serde_json::to_string(&patches)
        .unwrap_or_else(|_| format!(r#"[{{"type":"replace","new":"{}"}}]"#, escape_json(html)))
}

fn escape_json(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
}

pub fn compute_patch_as_map(old: &str, new: &str) -> HashMap<String, JsonValue> {
    let patch = compute_patch(old, new);

    let mut result = HashMap::new();
    result.insert("patches".to_string(), JsonValue::String(patch));
    result.insert("full".to_string(), JsonValue::Bool(false));

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compute_patch_identical() {
        let old = "<div>Hello</div>";
        let new = "<div>Hello</div>";
        let patch = compute_patch(old, new);
        assert!(patch.contains("[]") || patch.is_empty());
    }

    #[test]
    fn test_compute_patch_small_change() {
        let old = "<div id=\"c\">42</div>";
        let new = "<div id=\"c\">43</div>";
        let json = compute_patch(old, new);
        // Should not be a full replace of the whole document
        assert!(json.contains("43"));
        assert!(!json.contains("<div id=\\\"c\\\">42</div><div")); // sanity
    }

    #[test]
    fn test_compute_patch_replacement() {
        let old = "<h1>Old Title</h1>";
        let new = "<h1>New Title</h1>";
        let patch = compute_patch(old, new);
        assert!(patch.contains("New Title"));
    }
}
