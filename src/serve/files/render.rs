//! Rendering a `.slv` / `.erb` template found in the served folder.
//!
//! This is the one thing file mode cannot answer on the async side: templates
//! are code, and the interpreter is `!Send`, so the request goes through the
//! ordinary worker queue with [`RequestData::file_template`] set and lands
//! here instead of in the router.
//!
//! Rendered without a layout. A plain directory has no `layouts/application`,
//! and silently wrapping someone's standalone page in one they never wrote
//! would be a surprise; a template that wants a layout can `partial()` one in.

use std::cell::RefCell;
use std::rc::Rc;

use crate::interpreter::builtins::template::{get_template_cache, inject_template_helpers};
use crate::interpreter::value::{HashKey, HashPairs, Value};

use super::super::{RequestData, ResponseData};

/// Render `data.file_template`, or an error page describing why it failed.
pub(crate) fn render(data: &RequestData, relative: &str) -> ResponseData {
    let locals = locals_for(data);

    let cache = match get_template_cache() {
        Ok(cache) => cache,
        Err(error) => return failure(relative, &error),
    };

    inject_template_helpers(&locals);

    // File mode has no dev bar to read the `<!--solidev:view:…-->` comments,
    // so they would just be litter in the page the author wrote.
    let rendered =
        crate::template::without_dev_markers(|| cache.render(relative, &locals, Some(None)));

    match rendered {
        Ok(body) => ResponseData {
            status: 200,
            headers: vec![(
                "Content-Type".to_string(),
                "text/html; charset=utf-8".to_string(),
            )],
            body: body.into_bytes(),
        },
        Err(error) => failure(relative, &error),
    }
}

/// The locals every file-mode template gets: the request path and its query
/// parameters. No models, no session, no controller instance — there is no app
/// here to have any of those.
fn locals_for(data: &RequestData) -> Value {
    let mut params: HashPairs = HashPairs::default();
    for (key, value) in &data.query {
        params.insert(
            HashKey::String(key.as_str().into()),
            Value::String(value.as_str().into()),
        );
    }

    let mut locals: HashPairs = HashPairs::default();
    locals.insert(
        HashKey::String("path".into()),
        Value::String(data.path.as_str().into()),
    );
    locals.insert(
        HashKey::String("params".into()),
        Value::Hash(Rc::new(RefCell::new(params))),
    );

    Value::Hash(Rc::new(RefCell::new(locals)))
}

/// A template that fails to render reports the reason: in file mode the person
/// reading the page is the person who wrote the template.
fn failure(relative: &str, error: &str) -> ResponseData {
    let body = format!(
        "<!doctype html><meta charset=\"utf-8\">\
<title>Template error · {name}</title>\
<link rel=\"stylesheet\" href=\"/__soli/files.css\">\
<div class=\"main\"><div class=\"state\">\
<p class=\"lead\">Could not render <code>{name}</code>.</p>\
<pre class=\"code\"><code>{error}</code></pre>\
</div></div>",
        name = crate::template::renderer::html_escape(relative),
        error = crate::template::renderer::html_escape(error),
    );
    ResponseData {
        status: 500,
        headers: vec![(
            "Content-Type".to_string(),
            "text/html; charset=utf-8".to_string(),
        )],
        body: body.into_bytes(),
    }
}
