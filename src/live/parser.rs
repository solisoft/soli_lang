//! LiveView directive parser.

use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct LiveDirective {
    pub event: String,
    pub handler: String,
    pub target: Option<String>,
    pub values: HashMap<String, String>,
    pub original: String,
}

pub fn parse_live_directives(attrs: &[(String, String)]) -> Vec<LiveDirective> {
    let mut directives = Vec::new();

    for (name, value) in attrs {
        if let Some(directive) = parse_single_directive(name, value) {
            directives.push(directive);
        }
    }

    directives
}

fn parse_single_directive(attr_name: &str, attr_value: &str) -> Option<LiveDirective> {
    match attr_name {
        "soli-click" => Some(LiveDirective {
            event: "click".to_string(),
            handler: attr_value.to_string(),
            target: None,
            values: HashMap::new(),
            original: format!("{}={}", attr_name, attr_value),
        }),
        "soli-submit" => Some(LiveDirective {
            event: "submit".to_string(),
            handler: attr_value.to_string(),
            target: None,
            values: HashMap::new(),
            original: format!("{}={}", attr_name, attr_value),
        }),
        "soli-change" => Some(LiveDirective {
            event: "change".to_string(),
            handler: attr_value.to_string(),
            target: None,
            values: HashMap::new(),
            original: format!("{}={}", attr_name, attr_value),
        }),
        "soli-target" => Some(LiveDirective {
            event: "target".to_string(),
            handler: attr_value.to_string(),
            target: Some(attr_value.to_string()),
            values: HashMap::new(),
            original: format!("{}={}", attr_name, attr_value),
        }),
        _ if attr_name.starts_with("soli-value-") => {
            let key = attr_name.strip_prefix("soli-value-").unwrap().to_string();
            Some(LiveDirective {
                event: "value".to_string(),
                handler: key.clone(),
                target: None,
                values: vec![(key, attr_value.to_string())].into_iter().collect(),
                original: format!("{}={}", attr_name, attr_value),
            })
        }
        _ => None,
    }
}

pub fn is_live_directive(attr_name: &str) -> bool {
    matches!(
        attr_name,
        "soli-click" | "soli-submit" | "soli-change" | "soli-target"
    ) || attr_name.starts_with("soli-value-")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_click_directive() {
        let directives =
            parse_live_directives(&[("soli-click".to_string(), "increment".to_string())]);
        assert_eq!(directives.len(), 1);
        assert_eq!(directives[0].event, "click");
        assert_eq!(directives[0].handler, "increment");
    }

    #[test]
    fn test_is_live_directive() {
        assert!(is_live_directive("soli-click"));
        assert!(is_live_directive("soli-value-step"));
        assert!(!is_live_directive("class"));
    }
}
