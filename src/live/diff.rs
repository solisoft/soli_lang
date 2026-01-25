//! LiveView diff engine.

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

pub fn compute_patch(old_html: &str, new_html: &str) -> String {
    // For now, always do full replacement for reliability
    // Line-based diffing can be added back once client-side patching is more robust
    if old_html == new_html {
        return "[]".to_string();
    }
    compute_full_patch(new_html)
}

fn compute_full_patch(html: &str) -> String {
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
    fn test_compute_patch_replacement() {
        let old = "<h1>Old Title</h1>";
        let new = "<h1>New Title</h1>";
        let patch = compute_patch(old, new);
        assert!(patch.contains("New Title"));
    }
}
