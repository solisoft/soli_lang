//! The `--dev` mail inbox at `/__soli/inbox`: every message the app delivered
//! since the server started, searchable and paginated, with the HTML body, the
//! text body, and the raw MIME for each one.
//!
//! `/__soli/mailers` previews templates with fake data; this shows what the app
//! actually sent, with real data — including mail that never left the box
//! because no SMTP server is configured locally (see
//! [`crate::interpreter::builtins::mail_outbox`], which the mailer writes to).
//! Dev-only: wired only under `--dev`, and the store is empty otherwise.

use hyper::{Response, StatusCode};

use crate::interpreter::builtins::mail_outbox::{self, CapturedMail, Status};
use crate::interpreter::builtins::server::parse_query_string;

use super::{dev_bar, full, html_ok, Bytes, ResponseBody};

/// Messages per page before the `per` query parameter overrides it.
const DEFAULT_PER_PAGE: usize = 25;

/// Dark, dev-bar-styled page chrome, matching the DB browser and catalogs.
fn inbox_page(body: &str) -> String {
    format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><title>Soli \u{b7} Inbox</title>\
<meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">\
<style>body{{margin:0;font-family:'JetBrains Mono',ui-monospace,monospace;background:#08090b;color:#c9d1d9;padding:1.5rem;}}\
h1{{font-size:14px;letter-spacing:0.08em;color:#8b949e;font-weight:600;margin:0 0 0.75rem;}}\
a{{color:#8be9fd;text-decoration:none;}}a:hover{{text-decoration:underline;}}\
table{{border-collapse:collapse;width:100%;font-size:11px;}}\
th,td{{border:1px solid #30363d;padding:0.35rem 0.5rem;text-align:left;vertical-align:top;}}\
th{{background:#0b0d0f;color:#8b949e;}}tr:hover td{{background:#0e1013;}}\
pre{{background:#0b0d0f;border:1px solid #30363d;border-radius:6px;padding:0.75rem;overflow:auto;font-size:12px;white-space:pre-wrap;word-break:break-word;max-height:60vh;}}\
input,select{{background:#0b0d0f;color:#c9d1d9;border:1px solid #30363d;border-radius:6px;padding:0.35rem 0.5rem;font:inherit;}}\
button{{background:#1f6feb;color:#fff;border:0;border-radius:6px;padding:0.4rem 0.9rem;font:inherit;cursor:pointer;}}\
button.ghost{{background:transparent;color:#8b949e;border:1px solid #30363d;}}\
iframe{{width:100%;height:60vh;border:1px solid #30363d;border-radius:6px;background:#fff;}}\
.muted{{color:#8b949e;font-size:11px;}}.err{{color:#ff6b6b;}}\
.bar{{display:flex;flex-wrap:wrap;align-items:center;gap:0.5rem;margin:0 0 0.75rem;}}\
.grow{{flex:1 1 auto;}}.sent{{color:#b8e986;}}.captured{{color:#f0c674;}}.failed{{color:#ff6b6b;}}\
.subj{{color:#e6e6e6;}}.tag{{border:1px solid #30363d;border-radius:999px;padding:0.05rem 0.5rem;font-size:10px;}}\
</style></head><body><h1><a href=\"/__soli/inbox\">SOLI \u{b7} INBOX</a></h1>{body}</body></html>"
    )
}

/// Escaped, `—` when empty — the inbox shows a placeholder rather than a hole.
fn cell(value: &str) -> String {
    if value.trim().is_empty() {
        "<span class=\"muted\">\u{2014}</span>".to_string()
    } else {
        dev_bar::html_escape(value)
    }
}

/// `1.2 KB` / `340 B` for attachment sizes.
fn human_size(bytes: usize) -> String {
    const KB: f64 = 1024.0;
    let bytes = bytes as f64;
    if bytes < KB {
        format!("{} B", bytes as usize)
    } else if bytes < KB * KB {
        format!("{:.1} KB", bytes / KB)
    } else {
        format!("{:.1} MB", bytes / (KB * KB))
    }
}

/// `?q=&page=&per=` re-encoded for pagination links (page is supplied).
fn page_link(query: &str, per: usize, page: usize) -> String {
    format!(
        "/__soli/inbox?q={}&per={}&page={}",
        urlencoding::encode(query),
        per,
        page
    )
}

/// The page's slice of `matches`, plus the clamped page index. A page past the
/// end lands on the last one rather than showing an empty table.
fn paginate(total: usize, per: usize, page: usize) -> (usize, usize, usize) {
    let pages = total.div_ceil(per).max(1);
    let page = page.min(pages - 1);
    let start = page * per;
    (page, start, (start + per).min(total))
}

/// `GET /__soli/inbox` — searchable, paginated list of delivered mail.
pub(crate) fn handle_index(query: Option<&str>) -> Response<ResponseBody> {
    let params = query.map(parse_query_string).unwrap_or_default();
    let needle = params.get("q").cloned().unwrap_or_default();
    let per = params
        .get("per")
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(DEFAULT_PER_PAGE)
        .clamp(5, 200);
    let requested_page = params
        .get("page")
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(0);

    let matches = mail_outbox::search(&needle);
    let total = matches.len();
    let (page, start, end) = paginate(total, per, requested_page);
    let pages = total.div_ceil(per).max(1);

    let mut body = String::from(
        "<p class=\"muted\">Every mail this dev server delivered, newest first \u{b7} captured even with no SMTP \
configured \u{b7} kept in memory (last 100), cleared on restart.</p>",
    );

    // Search + clear toolbar. The search form carries `per` so a chosen page
    // size survives a query, and always resets to page 0.
    body.push_str(&format!(
        "<div class=\"bar\">\
<form method=\"get\" action=\"/__soli/inbox\" class=\"bar grow\" style=\"margin:0;\">\
<input type=\"search\" name=\"q\" value=\"{needle}\" placeholder=\"search subject, address, body or attachment\" \
style=\"flex:1 1 18rem;\" autofocus>\
<input type=\"hidden\" name=\"per\" value=\"{per}\">\
<button type=\"submit\">Search</button>\
{reset}\
</form>\
<a id=\"__soli_newmail\" href=\"/__soli/inbox\" class=\"tag\" style=\"display:none;color:#b8e986;\"></a>\
<form method=\"post\" action=\"/__soli/inbox/clear\" style=\"margin:0;\">\
<button type=\"submit\" class=\"ghost\">Clear inbox</button></form>\
</div>",
        needle = dev_bar::html_escape(&needle),
        per = per,
        reset = if needle.trim().is_empty() {
            String::new()
        } else {
            "<a href=\"/__soli/inbox\" class=\"muted\">reset</a>".to_string()
        },
    ));

    if total == 0 {
        body.push_str(&if needle.trim().is_empty() {
            "<p class=\"muted\">No mail yet. Send one with <code>UserMailer.welcome(user).deliver_now</code>, \
or preview the templates at <a href=\"/__soli/mailers\">/__soli/mailers</a>.</p>"
                .to_string()
        } else {
            format!(
                "<p class=\"muted\">No message matches <b>{}</b>.</p>",
                dev_bar::html_escape(&needle)
            )
        });
        return html_ok(inbox_page(&body));
    }

    body.push_str(&format!(
        "<p class=\"muted\">{total} message(s){filtered} \u{b7} showing {first}\u{2013}{last} \u{b7} page {page} of {pages}</p>",
        total = total,
        filtered = if needle.trim().is_empty() {
            String::new()
        } else {
            format!(" matching <b>{}</b>", dev_bar::html_escape(&needle))
        },
        first = start + 1,
        last = end,
        page = page + 1,
        pages = pages,
    ));

    body.push_str(
        "<div style=\"overflow-x:auto;\"><table><thead><tr>\
<th style=\"width:9rem;\">Date</th><th style=\"width:5rem;\">Status</th><th>Subject</th>\
<th style=\"width:14rem;\">From</th><th style=\"width:16rem;\">To</th></tr></thead><tbody>",
    );
    for mail in &matches[start..end] {
        body.push_str(&format!(
            "<tr><td class=\"muted\">{at}</td><td><span class=\"{status}\">{status}</span></td>\
<td><a class=\"subj\" href=\"/__soli/inbox/{id}\">{subject}</a>{attachments}</td>\
<td>{from}</td><td>{to}</td></tr>",
            at = dev_bar::html_escape(&mail.at),
            status = mail.status.label(),
            id = dev_bar::html_escape(&mail.id),
            subject = cell(&mail.subject),
            attachments = if mail.attachments.is_empty() {
                String::new()
            } else {
                format!(
                    " <span class=\"muted\">\u{1f4ce}{}</span>",
                    mail.attachments.len()
                )
            },
            from = cell(&mail.from),
            to = cell(&mail.recipients().join(", ")),
        ));
    }
    body.push_str("</tbody></table></div>");

    let mut nav = String::new();
    if page > 0 {
        nav.push_str(&format!(
            "<a href=\"{}\">&larr; prev</a> ",
            page_link(&needle, per, page - 1)
        ));
    }
    if page + 1 < pages {
        nav.push_str(&format!(
            "<a href=\"{}\">next &rarr;</a>",
            page_link(&needle, per, page + 1)
        ));
    }
    if !nav.is_empty() {
        body.push_str(&format!("<p style=\"margin-top:0.75rem;\">{}</p>", nav));
    }

    // Poll for arrivals so a mail sent from another tab announces itself
    // without clobbering the search/page the reader is looking at. The baseline
    // is the global newest id, not this page's — mail filtered out of the
    // current view was already here and isn't news.
    body.push_str(&format!(
        "<script>(function(){{var seen={latest};var badge=document.getElementById('__soli_newmail');\
if(!badge)return;setInterval(function(){{fetch('/__soli/inbox/count').then(function(r){{return r.json();}})\
.then(function(d){{if(d.latest>seen){{badge.style.display='inline';\
badge.textContent=(d.latest-seen)+' new \u{b7} reload';}}}}).catch(function(){{}});}},3000);}})();</script>",
        latest = mail_outbox::latest_id(),
    ));

    html_ok(inbox_page(&body))
}

/// `GET /__soli/inbox/count` — `{"count":N,"latest":ID}` for the arrivals badge.
pub(crate) fn handle_count() -> Response<ResponseBody> {
    let latest = mail_outbox::latest_id();
    Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/json")
        .body(full(Bytes::from(format!(
            "{{\"count\":{},\"latest\":{}}}",
            mail_outbox::count(),
            latest
        ))))
        .unwrap()
}

/// `POST /__soli/inbox/clear` — empty the inbox, then back to the listing.
pub(crate) fn handle_clear() -> Response<ResponseBody> {
    mail_outbox::clear();
    Response::builder()
        .status(StatusCode::SEE_OTHER)
        .header("Location", "/__soli/inbox")
        .body(full(Bytes::new()))
        .unwrap()
}

/// A 404 inbox page: an id that was cleared or aged out, or a view this message
/// has nothing to show for.
fn inbox_error(message: &str) -> Response<ResponseBody> {
    Response::builder()
        .status(StatusCode::NOT_FOUND)
        .header("Content-Type", "text/html; charset=utf-8")
        .body(full(Bytes::from(inbox_page(&format!(
            "<p class=\"err\">{}</p><p><a href=\"/__soli/inbox\">back to the inbox</a></p>",
            dev_bar::html_escape(message)
        )))))
        .unwrap()
}

/// The common case: the id doesn't resolve to a stored message.
fn unknown_message() -> Response<ResponseBody> {
    inbox_error("No such message — it was cleared, or aged out of the buffer.")
}

/// Dispatch everything under `/__soli/inbox/`: `<id>`, `<id>/html`, `<id>/text`,
/// `<id>/eml`, and `count`. Ids are decimal, so no path can escape the store.
pub(crate) fn handle_message(rest: &str) -> Response<ResponseBody> {
    if rest == "count" {
        return handle_count();
    }
    let (id, view) = match rest.split_once('/') {
        Some((id, view)) => (id, view),
        None => (rest, ""),
    };
    if id.is_empty() || !id.bytes().all(|b| b.is_ascii_digit()) {
        return unknown_message();
    }
    let Some(mail) = mail_outbox::get(id) else {
        return unknown_message();
    };
    match view {
        "" => render_detail(&mail),
        // The HTML body verbatim, for the detail page's iframe. The iframe is
        // sandboxed, so a mail's own scripts never run against the dev origin.
        "html" => html_ok(mail.html.unwrap_or_else(|| {
            "<!doctype html><p style=\"font:13px system-ui;color:#666\">This message has no HTML body.</p>"
                .to_string()
        })),
        "text" => Response::builder()
            .status(StatusCode::OK)
            .header("Content-Type", "text/plain; charset=utf-8")
            .body(full(Bytes::from(
                mail.text.unwrap_or_else(|| "(no text body)".to_string()),
            )))
            .unwrap(),
        "eml" => match mail.mime {
            Some(mime) => Response::builder()
                .status(StatusCode::OK)
                .header("Content-Type", "message/rfc822")
                .header(
                    "Content-Disposition",
                    format!("attachment; filename=\"soli-mail-{}.eml\"", mail.id),
                )
                .body(full(Bytes::from(mime)))
                .unwrap(),
            None => inbox_error(
                "No raw MIME for this message — it couldn't be assembled (see the status on \
the message), or it was too large to retain.",
            ),
        },
        _ => inbox_error("Unknown view — try /html, /text or /eml."),
    }
}

/// One message: headers, attachments, and HTML / text / raw tabs.
fn render_detail(mail: &CapturedMail) -> Response<ResponseBody> {
    let mut body = format!(
        "<p class=\"muted\"><a href=\"/__soli/inbox\">inbox</a> / message #{id}</p>",
        id = dev_bar::html_escape(&mail.id)
    );

    let mut rows = String::new();
    let mut row = |label: &str, value: String| {
        rows.push_str(&format!(
            "<tr><th style=\"width:8rem;\">{}</th><td>{}</td></tr>",
            label, value
        ));
    };
    row("Date", cell(&mail.at));
    row(
        "Status",
        match &mail.status {
            Status::Failed(err) => format!(
                "<span class=\"failed\">failed</span> <span class=\"muted\">{}</span>",
                dev_bar::html_escape(err)
            ),
            Status::Sent => "<span class=\"sent\">sent</span> \
<span class=\"muted\">accepted by the SMTP server</span>"
                .to_string(),
            Status::Captured => "<span class=\"captured\">captured</span> \
<span class=\"muted\">never sent \u{2014} no SMTP configured, or a test/logger delivery method</span>"
                .to_string(),
        },
    );
    row("Subject", cell(&mail.subject));
    row("From", cell(&mail.from));
    row("To", cell(&mail.to.join(", ")));
    if !mail.cc.is_empty() {
        row("Cc", cell(&mail.cc.join(", ")));
    }
    if !mail.bcc.is_empty() {
        row("Bcc", cell(&mail.bcc.join(", ")));
    }
    if let Some(reply_to) = &mail.reply_to {
        row("Reply-To", cell(reply_to));
    }
    if !mail.attachments.is_empty() {
        let list = mail
            .attachments
            .iter()
            .map(|a| {
                format!(
                    "{} <span class=\"muted\">({}, {})</span>",
                    dev_bar::html_escape(&a.filename),
                    dev_bar::html_escape(&a.content_type),
                    human_size(a.size)
                )
            })
            .collect::<Vec<_>>()
            .join("<br>");
        row("Attachments", list);
    }
    body.push_str(&format!("<table>{}</table>", rows));

    // Only offer the tabs this message actually has a body for.
    let mut tabs: Vec<(&str, &str)> = Vec::new();
    if mail.html.is_some() {
        tabs.push(("html", "HTML"));
    }
    if mail.text.is_some() {
        tabs.push(("text", "Text"));
    }
    if mail.mime.is_some() {
        tabs.push(("raw", "Raw"));
    }
    if tabs.is_empty() {
        body.push_str(
            "<p class=\"muted\" style=\"margin-top:1rem;\">This message has no body.</p>",
        );
        return html_ok(inbox_page(&body));
    }

    body.push_str("<div class=\"bar\" style=\"margin:1rem 0 0.5rem;\">");
    for (key, label) in &tabs {
        body.push_str(&format!(
            "<button class=\"ghost __soli_tab\" data-tab=\"{key}\">{label}</button>"
        ));
    }
    if mail.mime.is_some() {
        body.push_str(&format!(
            "<span class=\"grow\"></span><a href=\"/__soli/inbox/{}/eml\">download .eml</a>",
            dev_bar::html_escape(&mail.id)
        ));
    }
    body.push_str("</div>");

    if mail.html.is_some() {
        body.push_str(&format!(
            "<div class=\"__soli_pane\" data-pane=\"html\">\
<iframe src=\"/__soli/inbox/{}/html\" sandbox title=\"HTML body\"></iframe></div>",
            dev_bar::html_escape(&mail.id)
        ));
    }
    if let Some(text) = &mail.text {
        body.push_str(&format!(
            "<div class=\"__soli_pane\" data-pane=\"text\" style=\"display:none;\"><pre>{}</pre></div>",
            dev_bar::html_escape(text)
        ));
    }
    if let Some(mime) = &mail.mime {
        body.push_str(&format!(
            "<div class=\"__soli_pane\" data-pane=\"raw\" style=\"display:none;\"><pre>{}</pre></div>",
            dev_bar::html_escape(mime)
        ));
    }

    body.push_str(
        "<script>(function(){var tabs=document.querySelectorAll('.__soli_tab');\
var panes=document.querySelectorAll('.__soli_pane');\
function show(key){panes.forEach(function(p){p.style.display=p.getAttribute('data-pane')===key?'':'none';});\
tabs.forEach(function(t){var on=t.getAttribute('data-tab')===key;t.style.color=on?'#e6e6e6':'#8b949e';\
t.style.borderColor=on?'#8be9fd':'#30363d';});}\
tabs.forEach(function(t){t.addEventListener('click',function(){show(t.getAttribute('data-tab'));});});\
if(tabs.length)show(tabs[0].getAttribute('data-tab'));})();</script>",
    );

    html_ok(inbox_page(&body))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paginate_slices_pages_in_order() {
        assert_eq!(paginate(10, 4, 0), (0, 0, 4));
        assert_eq!(paginate(10, 4, 1), (1, 4, 8));
        assert_eq!(paginate(10, 4, 2), (2, 8, 10));
    }

    #[test]
    fn paginate_clamps_a_page_past_the_end() {
        assert_eq!(paginate(10, 4, 99), (2, 8, 10));
    }

    #[test]
    fn paginate_handles_an_empty_store() {
        assert_eq!(paginate(0, 25, 3), (0, 0, 0));
    }

    #[test]
    fn page_link_round_trips_the_search_term() {
        assert_eq!(
            page_link("a b&c", 25, 2),
            "/__soli/inbox?q=a%20b%26c&per=25&page=2"
        );
    }

    #[test]
    fn human_size_scales_units() {
        assert_eq!(human_size(512), "512 B");
        assert_eq!(human_size(2048), "2.0 KB");
        assert_eq!(human_size(3 * 1024 * 1024), "3.0 MB");
    }

    #[test]
    fn cell_falls_back_to_a_dash() {
        assert!(cell("  ").contains('\u{2014}'));
        assert_eq!(cell("a<b"), "a&lt;b");
    }
}
