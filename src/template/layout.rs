//! Layout support for templates.
//!
//! Handles wrapping rendered content with layout templates that use `<%= yield %>`.
//! The request path renders the view and its layout with one interpreter.
//! Writes directly into a shared output buffer (no intermediate String allocations).

use crate::interpreter::executor::Interpreter;
use crate::interpreter::value::Value;
use crate::template::parser::TemplateNode;

/// Type alias for partial renderer callback to reduce type complexity.
type PartialRenderer<'a> = Option<&'a dyn Fn(&str, &Value) -> Result<String, String>>;

/// Render layout nodes with an existing interpreter (avoids creating a new one).
/// Used to share a single interpreter across view + layout rendering.
pub fn render_layout_with_interpreter(
    interpreter: &mut Interpreter,
    nodes: &[TemplateNode],
    content: &str,
    data: &Value,
    partial_renderer: PartialRenderer<'_>,
    layout_path: Option<&str>,
) -> Result<String, String> {
    // Pre-allocate: layout wraps content, so output ≈ content + layout overhead
    let mut output = String::with_capacity(content.len() + 2048);
    crate::template::renderer::render_walker(
        interpreter,
        nodes,
        data,
        partial_renderer,
        layout_path,
        &mut output,
        crate::template::renderer::YieldMode::Layout { content },
    )?;
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interpreter::value::{HashKey, HashPairs};
    use crate::template::core_eval;
    use crate::template::parser::parse_template;
    use std::cell::RefCell;
    use std::rc::Rc;

    /// Parse `layout` and render it around `content` the way a request does:
    /// one template interpreter, through `render_layout_with_interpreter`.
    fn render_with_layout(
        layout: &str,
        content: &str,
        data: &Value,
        partial_renderer: PartialRenderer<'_>,
    ) -> Result<String, String> {
        let nodes = parse_template(layout)?;
        let mut interpreter = core_eval::create_template_interpreter(data);
        render_layout_with_interpreter(
            &mut interpreter,
            &nodes,
            content,
            data,
            partial_renderer,
            None,
        )
    }

    fn make_hash(pairs: Vec<(&str, Value)>) -> Value {
        let hash: HashPairs = pairs
            .into_iter()
            .map(|(k, v)| (HashKey::String(k.to_string().into()), v))
            .collect();
        Value::Hash(Rc::new(RefCell::new(hash)))
    }

    #[test]
    fn test_render_with_layout() {
        let layout = "<!DOCTYPE html><html><body><%= yield %></body></html>";
        let content = "<h1>Hello</h1>";
        let data = make_hash(vec![]);

        let result = render_with_layout(layout, content, &data, None).unwrap();
        assert_eq!(
            result,
            "<!DOCTYPE html><html><body><h1>Hello</h1></body></html>"
        );
    }

    #[test]
    fn test_layout_with_variables() {
        let layout = "<!DOCTYPE html><title><%= title %></title><body><%= yield %></body>";
        let content = "Page content";
        let data = make_hash(vec![("title", Value::String("My Page".into()))]);

        let result = render_with_layout(layout, content, &data, None).unwrap();
        assert_eq!(
            result,
            "<!DOCTYPE html><title>My Page</title><body>Page content</body>"
        );
    }

    #[test]
    fn test_layout_with_conditional() {
        let layout = "<% if show_nav %><nav>Nav</nav><% end %><%= yield %>";
        let content = "Content";
        let data = make_hash(vec![("show_nav", Value::Bool(true))]);

        let result = render_with_layout(layout, content, &data, None).unwrap();
        assert_eq!(result, "<nav>Nav</nav>Content");
    }

    #[test]
    fn test_named_yield_renders_captured_content() {
        use crate::template::content_store;
        let _frame = content_store::ensure_frame();
        content_store::append("head", "<script src=\"/a.js\"></script>");

        let layout = "<head><%= yield \"head\" %></head><body><%= yield %></body>";
        let result = render_with_layout(layout, "<h1>Hi</h1>", &make_hash(vec![]), None).unwrap();
        assert_eq!(
            result,
            "<head><script src=\"/a.js\"></script></head><body><h1>Hi</h1></body>"
        );
    }

    #[test]
    fn test_named_yield_missing_renders_empty() {
        use crate::template::content_store;
        let _frame = content_store::ensure_frame();

        let layout = "<head><%= yield \"head\" %></head><%= yield %>";
        let result = render_with_layout(layout, "Body", &make_hash(vec![]), None).unwrap();
        assert_eq!(result, "<head></head>Body");
    }

    #[test]
    fn test_named_yield_no_double_escape() {
        use crate::template::content_store;
        let _frame = content_store::ensure_frame();
        // Captured fragments are already-rendered HTML — the escaped `<%= %>`
        // form must still splice them verbatim, like the main content.
        content_store::append("head", "<meta content=\"a&b\">");

        let layout = "<%= yield \"head\" %>|<%- yield \"head\" %>";
        let result = render_with_layout(layout, "", &make_hash(vec![]), None).unwrap();
        assert_eq!(result, "<meta content=\"a&b\">|<meta content=\"a&b\">");
    }

    #[test]
    fn test_layout_side_capture_before_yield() {
        use crate::template::content_store;
        let _frame = content_store::ensure_frame();

        let layout = "<% content_for \"foot\" do %><footer></footer><% end %><%= yield \"foot\" %>";
        let result = render_with_layout(layout, "", &make_hash(vec![]), None).unwrap();
        assert_eq!(result, "<footer></footer>");
    }

    #[test]
    fn layout_escapes_non_primitive_display_values() {
        // Regression: a non-primitive `<%= %>` value (Instance/Class/DateTime)
        // must be HTML-escaped in layout output, matching the view renderer.
        // Before the walker was unified, layout wrote these raw — an XSS gap.
        use crate::interpreter::value::{Class, Instance};
        let class = Rc::new(Class {
            name: "Widget".to_string(),
            ..Default::default()
        });
        let widget = Value::Instance(Rc::new(RefCell::new(Instance::new(class))));
        let layout = "<body><%= widget %></body>";
        let data = make_hash(vec![("widget", widget)]);
        let result = render_with_layout(layout, "", &data, None).unwrap();
        assert_eq!(result, "<body>&lt;Widget instance&gt;</body>");
    }
}
