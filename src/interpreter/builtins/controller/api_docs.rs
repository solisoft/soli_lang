//! OpenAPI doc comments on controller actions.
//!
//! A comment block directly above an action becomes its OpenAPI description
//! when — and only when — it holds at least one known `@tag` line, so an
//! ordinary comment (`# GET /posts — …`, `# @posts is set above`) never leaks
//! into the spec:
//!
//! ```text
//! # Create a post.                        first text line → summary
//! # Notifies followers.                   the rest → description
//! # @query  draft  Bool   Save as draft
//! # @header X-Request-Id String! Idempotency key   (`!` = required)
//! # @body {"title": "Hello"}              JSON example; may continue on the next `#` lines
//! # @response 201 {"_key": "42"}
//! # @response 422 Validation failed       text → description only
//! def create
//! ```
//!
//! Without a `@body`, a `permit(params, {…})` whitelist in the action (or in
//! the `_permit_params` helper it calls) describes the request body.
//!
//! This is read from the controller's source text by
//! `registry::parse_controller_source`, which runs at boot, on every `--dev`
//! reload and at `soli build --protect` time — so bundles that ship no source
//! still carry their docs. Examples and schemas are stored as JSON strings so
//! the struct serializes in any format; `serve::openapi` turns them into the
//! spec. Docs are best-effort: nothing here can fail a boot.

use std::collections::HashMap;

use crate::ast::{ExprKind, StmtKind};

/// The tags a doc block understands. Anything else is reported by the
/// `docs/openapi` lint rule and otherwise ignored.
pub const KNOWN_TAGS: [&str; 8] = [
    "param",
    "query",
    "header",
    "body",
    "response",
    "tag",
    "deprecated",
    "hidden",
];

#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ActionDoc {
    pub summary: Option<String>,
    pub description: Option<String>,
    /// `@param`: descriptions/types for the route's path parameters.
    pub params: Vec<DocParam>,
    pub query: Vec<DocParam>,
    pub headers: Vec<DocParam>,
    /// `@body`: a JSON example (`example`) or, when it is not JSON, a description.
    pub body: Option<DocPayload>,
    /// `@response`, in the order written.
    pub responses: Vec<DocResponse>,
    /// `@tag`: replaces the controller-name tag.
    pub tag: Option<String>,
    pub deprecated: bool,
    /// `@hidden`: the operation is left out of the spec.
    pub hidden: bool,
    /// JSON Schema (as a string) inferred from `permit(params, {…})`; used
    /// only when there is no `@body`.
    pub permit_schema: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct DocParam {
    pub name: String,
    /// The Soli type as written (`Int`, `String`, …); see [`openapi_type`].
    pub kind: String,
    pub required: bool,
    pub description: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct DocPayload {
    /// Valid JSON text, when the tag's text parsed as JSON.
    pub example: Option<String>,
    pub description: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct DocResponse {
    pub code: String,
    pub payload: DocPayload,
}

/// A problem in a doc block, for `soli lint`. `line` is 1-based.
#[derive(Debug, Clone, PartialEq)]
pub struct DocIssue {
    pub line: usize,
    pub message: String,
}

/// The OpenAPI schema type for a Soli type name.
pub fn openapi_type(kind: &str) -> &'static str {
    match kind.trim_end_matches('!').to_ascii_lowercase().as_str() {
        "int" | "integer" => "integer",
        "float" | "number" | "decimal" => "number",
        "bool" | "boolean" => "boolean",
        "array" => "array",
        "hash" | "object" => "object",
        _ => "string",
    }
}

/// The `#`-comment text of `line`, or `None` when it is not a comment line.
fn comment_text(line: &str) -> Option<&str> {
    let rest = line.trim_start().strip_prefix('#')?;
    Some(rest.strip_prefix(' ').unwrap_or(rest))
}

/// The action named by a `def` line: `def show`, `def show(req)`,
/// `def ok? -> Bool`, `fn index(req) {`.
fn def_name(line: &str) -> Option<&str> {
    let t = line.trim_start();
    let rest = t.strip_prefix("def ").or_else(|| t.strip_prefix("fn "))?;
    let rest = rest.trim_start();
    let end = rest
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '?' || c == '!'))
        .unwrap_or(rest.len());
    let name = &rest[..end];
    (!name.is_empty()).then_some(name)
}

/// One `@tag` with its (possibly continued) text and the line it starts on.
struct TagLine {
    tag: String,
    text: String,
    line: usize,
}

/// Parse a comment block (the text of each `#` line, top to bottom).
/// `first_line` is the 1-based line of the block's first comment, for issues.
/// Returns `None` when the block has no tag line: it is an ordinary comment.
pub fn parse_doc_block(lines: &[&str], first_line: usize) -> Option<(ActionDoc, Vec<DocIssue>)> {
    let mut prose: Vec<&str> = Vec::new();
    let mut tags: Vec<TagLine> = Vec::new();
    for (i, raw) in lines.iter().enumerate() {
        let text = raw.trim();
        if let Some(rest) = text.strip_prefix('@') {
            let (tag, body) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
            tags.push(TagLine {
                tag: tag.to_string(),
                text: body.trim().to_string(),
                line: first_line + i,
            });
        } else if let Some(last) = tags.last_mut() {
            // A continuation of the tag above (multi-line JSON).
            if !text.is_empty() {
                last.text.push('\n');
                last.text.push_str(text);
            }
        } else {
            prose.push(text);
        }
    }
    // At least one *known* tag: `# @posts is loaded above` is prose about a
    // field, not a doc block.
    if !tags.iter().any(|t| KNOWN_TAGS.contains(&t.tag.as_str())) {
        return None;
    }

    let mut doc = ActionDoc::default();
    let mut issues = Vec::new();
    let paragraphs: Vec<&str> = prose.into_iter().skip_while(|l| l.is_empty()).collect();
    if let Some((first, rest)) = paragraphs.split_first() {
        doc.summary = Some(first.to_string());
        let description = rest.join("\n").trim().to_string();
        if !description.is_empty() {
            doc.description = Some(description);
        }
    }

    for t in tags {
        match t.tag.as_str() {
            "param" | "query" | "header" => match parse_param(&t.text) {
                Some(mut p) => {
                    if t.tag == "param" {
                        p.required = true;
                    }
                    match t.tag.as_str() {
                        "param" => doc.params.push(p),
                        "query" => doc.query.push(p),
                        _ => doc.headers.push(p),
                    }
                }
                None => issues.push(DocIssue {
                    line: t.line,
                    message: format!(
                        "`@{}` needs a name: `@{} <name> <Type> <description>`",
                        t.tag, t.tag
                    ),
                }),
            },
            "body" => {
                let (payload, bad_json) = parse_payload(&t.text);
                if bad_json {
                    issues.push(DocIssue {
                        line: t.line,
                        message: "`@body` looks like JSON but does not parse; it is used as a description".into(),
                    });
                }
                doc.body = Some(payload);
            }
            "response" => {
                let (code, rest) = t
                    .text
                    .split_once(char::is_whitespace)
                    .unwrap_or((&t.text, ""));
                if !is_status(code) {
                    issues.push(DocIssue {
                        line: t.line,
                        message: format!("`@response` needs a status code first, got `{}`", code),
                    });
                    continue;
                }
                let (payload, bad_json) = parse_payload(rest.trim());
                if bad_json {
                    issues.push(DocIssue {
                        line: t.line,
                        message: format!("`@response {}` looks like JSON but does not parse; it is used as a description", code),
                    });
                }
                doc.responses.push(DocResponse {
                    code: code.to_string(),
                    payload,
                });
            }
            "tag" => {
                if !t.text.is_empty() {
                    doc.tag = Some(t.text.clone());
                }
            }
            "deprecated" => doc.deprecated = true,
            "hidden" => doc.hidden = true,
            other => issues.push(DocIssue {
                line: t.line,
                message: format!(
                    "unknown doc tag `@{}` (known: {})",
                    other,
                    KNOWN_TAGS.map(|k| format!("@{k}")).join(", ")
                ),
            }),
        }
    }
    Some((doc, issues))
}

fn is_status(code: &str) -> bool {
    code == "default" || (code.len() == 3 && code.bytes().all(|b| b.is_ascii_digit() || b == b'X'))
}

/// `name Type! description…` — the type is optional (`String` by default).
fn parse_param(text: &str) -> Option<DocParam> {
    let mut words = text.split_whitespace();
    let name = words.next()?.to_string();
    let rest: Vec<&str> = words.collect();
    let (kind, desc_words) = match rest.first() {
        Some(w) if w.starts_with(|c: char| c.is_ascii_uppercase()) => (w.to_string(), &rest[1..]),
        _ => ("String".to_string(), &rest[..]),
    };
    let required = kind.ends_with('!');
    let description = desc_words.join(" ");
    Some(DocParam {
        name,
        kind: kind.trim_end_matches('!').to_string(),
        required,
        description: (!description.is_empty()).then_some(description),
    })
}

/// A payload is JSON when it starts like JSON and parses; otherwise text.
/// The flag reports text that starts like JSON but does not parse.
fn parse_payload(text: &str) -> (DocPayload, bool) {
    let t = text.trim();
    if t.is_empty() {
        return (DocPayload::default(), false);
    }
    if t.starts_with('{') || t.starts_with('[') {
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(t) {
            return (
                DocPayload {
                    example: Some(value.to_string()),
                    description: None,
                },
                false,
            );
        }
        return (
            DocPayload {
                example: None,
                description: Some(t.to_string()),
            },
            true,
        );
    }
    (
        DocPayload {
            example: None,
            description: Some(t.replace('\n', " ")),
        },
        false,
    )
}

/// Each method's `def` line (0-based index), in file order.
fn method_lines(lines: &[&str]) -> Vec<(usize, String)> {
    lines
        .iter()
        .enumerate()
        .filter_map(|(i, l)| def_name(l).map(|n| (i, n.to_string())))
        .collect()
}

/// The contiguous `#` lines directly above line `def_idx` (no blank line in
/// between), top to bottom, with the 0-based index of the first.
fn comment_block_above<'a>(lines: &[&'a str], def_idx: usize) -> (Vec<&'a str>, usize) {
    let mut start = def_idx;
    while start > 0 && comment_text(lines[start - 1]).is_some() {
        start -= 1;
    }
    let block = lines[start..def_idx]
        .iter()
        .filter_map(|l| comment_text(l))
        .collect();
    (block, start)
}

/// A method's code, without comment lines: from its `def` to the next `def`.
fn method_body(lines: &[&str], methods: &[(usize, String)], idx: usize) -> String {
    let from = methods[idx].0 + 1;
    let to = methods.get(idx + 1).map(|m| m.0).unwrap_or(lines.len());
    lines[from..to]
        .iter()
        .filter(|l| comment_text(l).is_none())
        .copied()
        .collect::<Vec<_>>()
        .join("\n")
}

/// Every public action's doc, keyed by action name, plus the issues found.
pub fn extract_action_docs_with_issues(
    source: &str,
) -> (HashMap<String, ActionDoc>, Vec<DocIssue>) {
    let lines: Vec<&str> = source.lines().collect();
    let methods = method_lines(&lines);
    let mut docs = HashMap::new();
    let mut issues = Vec::new();

    for (idx, (def_idx, name)) in methods.iter().enumerate() {
        if name.starts_with('_') {
            continue;
        }
        let (block, start) = comment_block_above(&lines, *def_idx);
        let mut doc = match parse_doc_block(&block, start + 1) {
            Some((doc, found)) => {
                issues.extend(found);
                doc
            }
            None => ActionDoc::default(),
        };
        if doc.body.is_none() {
            let body = method_body(&lines, &methods, idx);
            doc.permit_schema = permit_schema_in(&body).or_else(|| {
                // `@post = Post.create(@_permit_params(params))` — follow the helper.
                if !body.contains("_permit_params(") {
                    return None;
                }
                let helper = methods.iter().position(|(_, n)| n == "_permit_params")?;
                permit_schema_in(&method_body(&lines, &methods, helper))
            });
        }
        if doc != ActionDoc::default() {
            docs.insert(name.clone(), doc);
        }
    }
    (docs, issues)
}

/// Every public action's doc, keyed by action name.
pub fn extract_action_docs(source: &str) -> HashMap<String, ActionDoc> {
    extract_action_docs_with_issues(source).0
}

/// The schema of the first `permit(<x>, {…})` in `code`, as a JSON string.
fn permit_schema_in(code: &str) -> Option<String> {
    let at = code.find("permit(")?;
    let after = &code[at + "permit(".len()..];
    // The second argument: the first `{` at depth 0 after a top-level comma.
    let comma = top_level_comma(after)?;
    let hash_src = balanced_braces(after[comma + 1..].trim_start())?;
    let expr = parse_expression(hash_src)?;
    let schema = permit_hash_schema(&expr)?;
    Some(schema.to_string())
}

fn top_level_comma(s: &str) -> Option<usize> {
    let mut depth = 0i32;
    for (i, c) in s.char_indices() {
        match c {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => {
                if depth == 0 {
                    return None;
                }
                depth -= 1;
            }
            ',' if depth == 0 => return Some(i),
            _ => {}
        }
    }
    None
}

/// `s` starts with `{`: the text up to its matching `}` (strings respected).
fn balanced_braces(s: &str) -> Option<&str> {
    if !s.starts_with('{') {
        return None;
    }
    let mut depth = 0i32;
    let mut in_string = false;
    let mut escaped = false;
    for (i, c) in s.char_indices() {
        if in_string {
            match c {
                _ if escaped => escaped = false,
                '\\' => escaped = true,
                '"' => in_string = false,
                _ => {}
            }
            continue;
        }
        match c {
            '"' => in_string = true,
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(&s[..=i]);
                }
            }
            _ => {}
        }
    }
    None
}

fn parse_expression(src: &str) -> Option<crate::ast::Expr> {
    let wrapped = format!("let __permit_shape = {}", src);
    let tokens = crate::lexer::Scanner::new(&wrapped).scan_tokens().ok()?;
    let program = crate::parser::Parser::new(tokens).parse().ok()?;
    match program.statements.into_iter().next()?.kind {
        StmtKind::Let { initializer, .. } => initializer,
        _ => None,
    }
}

/// `permit`'s whitelist shape as a JSON Schema: `true` → any scalar,
/// `[]` → array, `[{…}]` → array of objects, `{…}` → object.
fn permit_hash_schema(expr: &crate::ast::Expr) -> Option<serde_json::Value> {
    use serde_json::{json, Map, Value};
    let ExprKind::Hash(pairs) = &expr.kind else {
        return None;
    };
    let mut props = Map::new();
    for (key, value) in pairs {
        let name = match &key.kind {
            ExprKind::StringLiteral(s) => s.clone(),
            ExprKind::Variable(s) => s.clone(),
            ExprKind::Symbol(s) => s.clone(),
            _ => continue,
        };
        let schema = match &value.kind {
            ExprKind::Hash(_) => permit_hash_schema(value).unwrap_or_else(|| json!({})),
            ExprKind::Array(items) => match items.first() {
                Some(inner) if matches!(inner.kind, ExprKind::Hash(_)) => json!({
                    "type": "array",
                    "items": permit_hash_schema(inner).unwrap_or_else(|| json!({}))
                }),
                _ => json!({ "type": "array", "items": {} }),
            },
            _ => json!({}),
        };
        props.insert(name, schema);
    }
    Some(json!({ "type": "object", "properties": Value::Object(props) }))
}

/// A JSON Schema inferred from an example value.
pub fn schema_from_example(value: &serde_json::Value) -> serde_json::Value {
    use serde_json::{json, Map, Value};
    match value {
        Value::Null => json!({ "nullable": true }),
        Value::Bool(_) => json!({ "type": "boolean" }),
        Value::Number(n) if n.is_i64() || n.is_u64() => json!({ "type": "integer" }),
        Value::Number(_) => json!({ "type": "number" }),
        Value::String(_) => json!({ "type": "string" }),
        Value::Array(items) => json!({
            "type": "array",
            "items": items.first().map(schema_from_example).unwrap_or_else(|| json!({}))
        }),
        Value::Object(map) => {
            let props: Map<String, Value> = map
                .iter()
                .map(|(k, v)| (k.clone(), schema_from_example(v)))
                .collect();
            json!({ "type": "object", "properties": props })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc_of(source: &str, action: &str) -> Option<ActionDoc> {
        extract_action_docs(source).get(action).cloned()
    }

    #[test]
    fn a_comment_without_a_tag_is_not_a_doc() {
        let src = "class PostsController < Controller\n  # GET /posts — lists them\n  def index\n  end\nend\n";
        assert!(doc_of(src, "index").is_none());
    }

    #[test]
    fn summary_description_and_every_tag() {
        let src = r#"class PostsController < Controller
  # Create a post.
  # Notifies followers.
  # Twice.
  # @query  draft  Bool   Save as draft
  # @header X-Request-Id String! Idempotency key
  # @body {"title": "Hello",
  #        "tags": ["soli"]}
  # @response 201 {"_key": "42"}
  # @response 422 Validation failed
  # @tag Blog
  # @deprecated
  def create(req)
  end
end
"#;
        let doc = doc_of(src, "create").expect("doc");
        assert_eq!(doc.summary.as_deref(), Some("Create a post."));
        assert_eq!(
            doc.description.as_deref(),
            Some("Notifies followers.\nTwice.")
        );
        assert_eq!(doc.query[0].name, "draft");
        assert_eq!(doc.query[0].kind, "Bool");
        assert!(!doc.query[0].required);
        assert_eq!(doc.headers[0].name, "X-Request-Id");
        assert!(doc.headers[0].required, "`!` marks it required");
        let example: serde_json::Value =
            serde_json::from_str(doc.body.as_ref().unwrap().example.as_deref().unwrap()).unwrap();
        assert_eq!(
            example,
            serde_json::json!({"title": "Hello", "tags": ["soli"]}),
            "the JSON continues on the next line"
        );
        assert_eq!(doc.responses[0].code, "201");
        assert!(doc.responses[0].payload.example.is_some());
        assert_eq!(
            doc.responses[1].payload.description.as_deref(),
            Some("Validation failed")
        );
        assert_eq!(doc.tag.as_deref(), Some("Blog"));
        assert!(doc.deprecated);
        assert!(doc.permit_schema.is_none(), "no permit call");
    }

    #[test]
    fn paren_less_def_and_path_params_are_required() {
        let src = "  # @param id Int  Post key\n  # @hidden\n  def show\n  end\n";
        let doc = doc_of(src, "show").expect("doc");
        assert_eq!(doc.params[0].kind, "Int");
        assert!(doc.params[0].required);
        assert_eq!(doc.params[0].description.as_deref(), Some("Post key"));
        assert!(doc.hidden);
    }

    #[test]
    fn a_comment_about_a_field_is_not_a_doc() {
        let src = "  # @posts is loaded by the before_action\n  def index\n  end\n";
        let (docs, issues) = extract_action_docs_with_issues(src);
        assert!(!docs.contains_key("index"));
        assert!(issues.is_empty(), "no lint noise on ordinary comments");
    }

    #[test]
    fn a_blank_line_detaches_the_comment() {
        let src = "  # @response 404 Not found\n\n  def show\n  end\n";
        assert!(doc_of(src, "show").is_none());
    }

    #[test]
    fn invalid_json_becomes_a_description_and_an_issue() {
        let src = "  # @body {\"title\": }\n  def create\n  end\n";
        let (docs, issues) = extract_action_docs_with_issues(src);
        let body = docs["create"].body.clone().unwrap();
        assert!(body.example.is_none());
        assert_eq!(body.description.as_deref(), Some("{\"title\": }"));
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].line, 1);
    }

    #[test]
    fn unknown_tags_and_bad_status_codes_are_issues() {
        let src = "  # @returns 200\n  # @response ok Fine\n  def index\n  end\n";
        let (_, issues) = extract_action_docs_with_issues(src);
        assert_eq!(issues.len(), 2, "{issues:?}");
        assert!(issues[0].message.contains("@returns"));
        assert!(issues[1].message.contains("status code"));
    }

    #[test]
    fn permit_in_the_action_describes_the_body() {
        let src = r#"class PostsController < Controller
  def create
    @post = Post.create(permit(params, {"title": true, "tags": [], "author": {"name": true}, "items": [{"sku": true}]}))
  end
end
"#;
        let doc = doc_of(src, "create").expect("inferred");
        let schema: serde_json::Value =
            serde_json::from_str(doc.permit_schema.as_deref().unwrap()).unwrap();
        let props = &schema["properties"];
        assert_eq!(props["title"], serde_json::json!({}));
        assert_eq!(props["tags"]["type"], "array");
        assert_eq!(props["author"]["properties"]["name"], serde_json::json!({}));
        assert_eq!(
            props["items"]["items"]["properties"]["sku"],
            serde_json::json!({})
        );
    }

    #[test]
    fn permit_through_the_scaffold_helper() {
        let src = r#"class PostsController < Controller
  # POST /posts
  def create
    @post = Post.create(this._permit_params(params))
  end

  def _permit_params(params)
    permit(params, {
      "title": true,
      "body": true
    })
  end
end
"#;
        let doc = doc_of(src, "create").expect("inferred through _permit_params");
        let schema: serde_json::Value =
            serde_json::from_str(doc.permit_schema.as_deref().unwrap()).unwrap();
        assert!(schema["properties"]["title"].is_object());
        assert!(schema["properties"]["body"].is_object());
        assert!(doc.summary.is_none(), "the plain comment stays out");
    }

    #[test]
    fn a_body_tag_wins_over_permit() {
        let src = "  # @body {\"title\": \"x\"}\n  def create\n    permit(params, {\"other\": true})\n  end\n";
        let doc = doc_of(src, "create").unwrap();
        assert!(doc.permit_schema.is_none());
        assert!(doc.body.is_some());
    }

    #[test]
    fn schema_from_example_types() {
        let v: serde_json::Value = serde_json::from_str(
            r#"{"id": 1, "price": 1.5, "ok": true, "tags": ["a"], "n": null}"#,
        )
        .unwrap();
        let s = schema_from_example(&v);
        assert_eq!(s["properties"]["id"]["type"], "integer");
        assert_eq!(s["properties"]["price"]["type"], "number");
        assert_eq!(s["properties"]["ok"]["type"], "boolean");
        assert_eq!(s["properties"]["tags"]["items"]["type"], "string");
    }

    #[test]
    fn old_controller_info_without_docs_still_deserializes() {
        let json = r#"{"name":"PostsController","class_name":"posts","actions":[],"before_actions":[],"after_actions":[],"layout":null,"action_layouts":[]}"#;
        let info: super::super::controller::ControllerInfo = serde_json::from_str(json).unwrap();
        assert!(info.docs.is_empty());
    }
}
