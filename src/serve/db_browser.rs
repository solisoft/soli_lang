//! The `--dev` SoliDB browser at `/__soli/db`: list collections, paginate a
//! collection's rows, view a document as JSON, and run a read-only SDBQL
//! query. Extracted from the serve god-module. Dev-only — wired only under
//! `--dev`, and mutating queries are rejected.

use hyper::Response;

use crate::interpreter::builtins::server::parse_query_string;

use super::{dev_bar, html_ok, ResponseBody};

/// Dark, dev-bar-styled page chrome for the DB browser (adds table CSS).
fn db_page(body: &str) -> String {
    format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><title>Soli \u{b7} Database</title>\
<style>body{{margin:0;font-family:'JetBrains Mono',ui-monospace,monospace;background:#08090b;color:#c9d1d9;padding:1.5rem;}}\
h1{{font-size:14px;letter-spacing:0.08em;color:#8b949e;font-weight:600;margin:0 0 0.75rem;}}\
a{{color:#8be9fd;text-decoration:none;}}a:hover{{text-decoration:underline;}}\
table{{border-collapse:collapse;width:100%;font-size:11px;}}\
th,td{{border:1px solid #30363d;padding:0.3rem 0.5rem;text-align:left;vertical-align:top;max-width:420px;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;}}\
th{{background:#0b0d0f;color:#8b949e;position:sticky;top:0;}}\
pre{{background:#0b0d0f;border:1px solid #30363d;border-radius:6px;padding:0.75rem;overflow:auto;font-size:12px;}}\
textarea{{width:100%;box-sizing:border-box;background:#0b0d0f;color:#c9d1d9;border:1px solid #30363d;border-radius:6px;padding:0.5rem;font:inherit;}}\
button{{background:#1f6feb;color:#fff;border:0;border-radius:6px;padding:0.4rem 0.9rem;font:inherit;cursor:pointer;}}\
.muted{{color:#8b949e;font-size:11px;}}.err{{color:#ff6b6b;}}</style></head>\
<body><h1><a href=\"/__soli/db\">SOLI \u{b7} DATABASE</a></h1>{body}</body></html>",
    )
}

/// Dev-only DB browser error page (message is escaped).
fn db_error_page(msg: &str) -> Response<ResponseBody> {
    html_ok(db_page(&format!(
        "<p class=\"err\">{}</p>",
        dev_bar::html_escape(msg)
    )))
}

/// Build a SoliDB client from the model DB config (auth + database), for the
/// dev browser. Mirrors `jobs::make_client`.
fn db_browser_client() -> Result<crate::solidb_http::SoliDBClient, String> {
    use crate::interpreter::builtins::model::core::{
        get_api_key, get_basic_auth, get_database_name, get_jwt_token, DB_CONFIG,
    };
    use crate::solidb_http::SoliDBClient;
    let host = &DB_CONFIG.host;
    let mut client =
        SoliDBClient::connect(host).map_err(|e| format!("SoliDB connect failed: {}", e))?;
    if let Some(jwt) = get_jwt_token() {
        client = client.with_jwt_token(&jwt);
    } else if let Some(key) = get_api_key() {
        client = client.with_api_key(key);
    } else if let Some(basic) = get_basic_auth() {
        if let Some(rest) = basic.strip_prefix("Basic ") {
            use base64::{engine::general_purpose::STANDARD, Engine as _};
            if let Ok(bytes) = STANDARD.decode(rest) {
                if let Ok(s) = String::from_utf8(bytes) {
                    if let Some((u, p)) = s.split_once(':') {
                        client = client.with_basic_auth(u, p);
                    }
                }
            }
        }
    }
    client.set_database(&get_database_name());
    Ok(client)
}

/// Non-system collection names, sorted. The caller wraps this in block_in_place.
fn db_list_collection_names() -> Result<Vec<String>, String> {
    let client = db_browser_client()?;
    let cols = client.list_collections().map_err(|e| e.to_string())?;
    let mut names: Vec<String> = cols
        .iter()
        .filter_map(|c| c.get("name").and_then(|v| v.as_str()).map(str::to_string))
        .filter(|n| !n.starts_with('_'))
        .collect();
    names.sort();
    names.dedup();
    Ok(names)
}

/// Reject non-read queries (a lexical guard for the dev query box, paired with
/// the collection allow-list; the endpoint is dev-gated regardless).
fn is_write_query(sql: &str) -> bool {
    let upper = sql.to_uppercase();
    ["INSERT", "UPDATE", "REPLACE", "REMOVE", "UPSERT"]
        .iter()
        .any(|kw| {
            upper
                .split(|c: char| !c.is_ascii_alphanumeric())
                .any(|tok| tok == *kw)
        })
}

/// A collection name safe to interpolate into a query (it can't be a bind var).
fn valid_collection_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// Escaped display text for a JSON cell (scalars bare, containers as JSON).
fn db_json_cell(v: &serde_json::Value) -> String {
    let s = match v {
        serde_json::Value::String(s) => s.clone(),
        other => other.to_string(),
    };
    dev_bar::html_escape(&s)
}

/// Render an array of row objects as an HTML table (union of keys as columns,
/// `_key`/`_rev` first). Shared by the collection view and query results.
fn db_rows_table(rows: &[serde_json::Value]) -> String {
    if rows.is_empty() {
        return "<p class=\"muted\">No rows.</p>".to_string();
    }
    let mut cols: Vec<String> = Vec::new();
    for row in rows {
        if let Some(obj) = row.as_object() {
            for k in obj.keys() {
                if !cols.contains(k) {
                    cols.push(k.clone());
                }
            }
        }
    }
    cols.sort_by_key(|k| match k.as_str() {
        "_key" => (0, k.clone()),
        "_rev" => (1, k.clone()),
        _ => (2, k.clone()),
    });
    let mut html = String::from("<div style=\"overflow-x:auto;\"><table><thead><tr>");
    for c in &cols {
        html.push_str(&format!("<th>{}</th>", dev_bar::html_escape(c)));
    }
    html.push_str("</tr></thead><tbody>");
    for row in rows {
        html.push_str("<tr>");
        let obj = row.as_object();
        for c in &cols {
            let content = obj
                .and_then(|o| o.get(c))
                .map(db_json_cell)
                .unwrap_or_default();
            html.push_str(&format!("<td title=\"{content}\">{content}</td>"));
        }
        html.push_str("</tr>");
    }
    html.push_str("</tbody></table></div>");
    html
}

/// `GET /__soli/db` — collections index + a read-only query box (runs `?q=`).
pub(crate) fn handle_db_index(query: Option<&str>) -> Response<ResponseBody> {
    use crate::interpreter::builtins::model::core::{get_database_name, DB_CONFIG};
    let params = query.map(parse_query_string).unwrap_or_default();

    let names = match tokio::task::block_in_place(db_list_collection_names) {
        Ok(n) => n,
        Err(e) => return db_error_page(&format!("Database unavailable: {}", e)),
    };

    let mut body = format!(
        "<p class=\"muted\">Database <b>{}</b> @ <code>{}</code> \u{b7} {} collections \u{b7} dev-only.</p>",
        dev_bar::html_escape(&get_database_name()),
        dev_bar::html_escape(&DB_CONFIG.host),
        names.len()
    );

    body.push_str("<div style=\"display:grid;grid-template-columns:repeat(auto-fill,minmax(200px,1fr));gap:0.4rem;margin:0.75rem 0 1.25rem;\">");
    for n in &names {
        let esc = dev_bar::html_escape(n);
        body.push_str(&format!(
            "<a href=\"/__soli/db/{esc}\" style=\"border:1px solid #30363d;border-radius:6px;padding:0.4rem 0.6rem;\">{esc}</a>"
        ));
    }
    body.push_str("</div>");

    let q = params.get("q").cloned().unwrap_or_default();
    let placeholder = names.first().map(|s| s.as_str()).unwrap_or("collection");
    body.push_str(&format!(
        "<form method=\"get\" action=\"/__soli/db\">\
<div class=\"muted\" style=\"margin-bottom:0.25rem;\">Read-only query (SDBQL) \u{2014} writes are rejected:</div>\
<textarea name=\"q\" rows=\"3\" placeholder=\"FOR d IN {} LIMIT 20 RETURN d\">{}</textarea>\
<div style=\"margin-top:0.4rem;\"><button type=\"submit\">Run</button></div></form>",
        dev_bar::html_escape(placeholder),
        dev_bar::html_escape(&q)
    ));

    if !q.trim().is_empty() {
        if is_write_query(&q) {
            body.push_str("<p class=\"err\" style=\"margin-top:1rem;\">Only read queries are allowed here (INSERT/UPDATE/REPLACE/REMOVE/UPSERT rejected).</p>");
        } else {
            let sql = q.clone();
            match tokio::task::block_in_place(move || {
                crate::interpreter::builtins::model::crud::exec_async_query_with_binds(sql, None)
            }) {
                Ok(rows) => {
                    body.push_str(&format!(
                        "<div class=\"muted\" style=\"margin-top:1rem;\">{} row(s)</div>",
                        rows.len()
                    ));
                    body.push_str(&db_rows_table(&rows));
                }
                Err(e) => body.push_str(&format!(
                    "<p class=\"err\" style=\"margin-top:1rem;\">{}</p>",
                    dev_bar::html_escape(&e)
                )),
            }
        }
    }

    html_ok(db_page(&body))
}

/// `GET /__soli/db/<collection>?page=N&size=M` — paginated rows.
pub(crate) fn handle_db_collection(coll: &str, query: Option<&str>) -> Response<ResponseBody> {
    if !valid_collection_name(coll) {
        return db_error_page("invalid collection name");
    }
    // Allow-list against the real collections: the name is interpolated into
    // the query (it can't be a bind var), so never trust it blindly.
    let known = match tokio::task::block_in_place(db_list_collection_names) {
        Ok(n) => n,
        Err(e) => return db_error_page(&format!("Database unavailable: {}", e)),
    };
    if !known.iter().any(|n| n == coll) {
        return db_error_page(&format!("unknown collection: {}", coll));
    }

    let params = query.map(parse_query_string).unwrap_or_default();
    let page: usize = params.get("page").and_then(|s| s.parse().ok()).unwrap_or(0);
    let size: usize = params
        .get("size")
        .and_then(|s| s.parse().ok())
        .unwrap_or(50)
        .clamp(1, 500);
    let offset = page.saturating_mul(size);

    let mut binds = std::collections::HashMap::new();
    binds.insert("off".to_string(), serde_json::json!(offset));
    binds.insert("cnt".to_string(), serde_json::json!(size));
    let sql = format!("FOR d IN {} LIMIT @off, @cnt RETURN d", coll);
    let rows = match tokio::task::block_in_place(move || {
        crate::interpreter::builtins::model::crud::exec_async_query_with_binds(sql, Some(binds))
    }) {
        Ok(r) => r,
        Err(e) => return db_error_page(&e),
    };

    let esc = dev_bar::html_escape(coll);
    let mut body = format!(
        "<p class=\"muted\"><a href=\"/__soli/db\">collections</a> / <b>{}</b> \u{b7} page {} \u{b7} {} row(s) \u{b7} open one at <code>/__soli/db/{}/&lt;_key&gt;</code></p>",
        esc,
        page,
        rows.len(),
        esc
    );
    let mut nav = String::new();
    if page > 0 {
        nav.push_str(&format!(
            "<a href=\"/__soli/db/{esc}?page={}&size={size}\">&larr; prev</a> ",
            page - 1
        ));
    }
    if rows.len() == size {
        nav.push_str(&format!(
            "<a href=\"/__soli/db/{esc}?page={}&size={size}\">next &rarr;</a>",
            page + 1
        ));
    }
    if !nav.is_empty() {
        body.push_str(&format!("<p>{}</p>", nav));
    }
    body.push_str(&db_rows_table(&rows));
    html_ok(db_page(&body))
}

/// `GET /__soli/db/<collection>/<key>` — one document as pretty JSON.
pub(crate) fn handle_db_document(coll: &str, key: &str) -> Response<ResponseBody> {
    if !valid_collection_name(coll) {
        return db_error_page("invalid collection name");
    }
    // Allow-list so a bogus URL can't trip exec_get's auto-create side effect.
    let known = match tokio::task::block_in_place(db_list_collection_names) {
        Ok(n) => n,
        Err(e) => return db_error_page(&format!("Database unavailable: {}", e)),
    };
    if !known.iter().any(|n| n == coll) {
        return db_error_page(&format!("unknown collection: {}", coll));
    }

    let coll_owned = coll.to_string();
    let key_owned = key.to_string();
    let doc = match tokio::task::block_in_place(move || {
        crate::interpreter::builtins::model::crud::exec_get(&coll_owned, &key_owned)
    }) {
        Ok(d) => d,
        Err(e) => return db_error_page(&e),
    };
    let pretty = serde_json::to_string_pretty(&doc).unwrap_or_else(|_| doc.to_string());
    let body = format!(
        "<p class=\"muted\"><a href=\"/__soli/db\">collections</a> / <a href=\"/__soli/db/{c}\">{c}</a> / <b>{k}</b></p><pre>{}</pre>",
        dev_bar::html_escape(&pretty),
        c = dev_bar::html_escape(coll),
        k = dev_bar::html_escape(key),
    );
    html_ok(db_page(&body))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn db_write_query_guard_rejects_mutations_only() {
        assert!(is_write_query("FOR d IN posts REMOVE d IN posts"));
        assert!(is_write_query("INSERT {a:1} INTO posts"));
        assert!(is_write_query("update posts set x=1")); // case-insensitive
        assert!(is_write_query("UPSERT {a:1} INSERT {} UPDATE {} IN c"));
        // Reads pass, including field/collection names that merely contain a
        // keyword as a substring (word-boundary check).
        assert!(!is_write_query("FOR d IN posts LIMIT 20 RETURN d"));
        assert!(!is_write_query("FOR d IN updates RETURN d")); // 'updates' != 'update'
        assert!(!is_write_query("FOR d IN posts RETURN d.inserted_at"));
    }

    #[test]
    fn db_collection_name_validation() {
        assert!(valid_collection_name("posts"));
        assert!(valid_collection_name("user_sessions-2"));
        assert!(!valid_collection_name(""));
        assert!(!valid_collection_name("posts; DROP")); // space/semicolon
        assert!(!valid_collection_name("a/b"));
        assert!(!valid_collection_name(&"x".repeat(200)));
    }

    #[test]
    fn db_rows_table_orders_key_columns_first() {
        let rows = vec![
            serde_json::json!({ "title": "a", "_key": "1", "_rev": "r1" }),
            serde_json::json!({ "title": "b", "_key": "2", "_rev": "r2", "extra": 9 }),
        ];
        let html = db_rows_table(&rows);
        let key_pos = html.find("<th>_key</th>").unwrap();
        let rev_pos = html.find("<th>_rev</th>").unwrap();
        let title_pos = html.find("<th>title</th>").unwrap();
        assert!(key_pos < rev_pos && rev_pos < title_pos);
        assert!(html.contains("<th>extra</th>")); // union of keys
        assert_eq!(db_rows_table(&[]), "<p class=\"muted\">No rows.</p>");
    }
}
