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

use super::operator_shell::{self, Section};
use super::{dev_bar, full, html_ok, Bytes, ResponseBody};

/// Messages per page before the `per` query parameter overrides it.
const DEFAULT_PER_PAGE: usize = 25;

const INTRO: &str =
    "Every mail this dev server delivered, newest first \u{b7} captured even with no \
SMTP configured \u{b7} the last 100, in memory, cleared on restart.";

fn inbox_page(body: &str) -> String {
    operator_shell::page(Section::Inbox, "Inbox", INTRO, body)
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

    let mut body = String::new();

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
<button type=\"submit\" class=\"danger\">Clear inbox</button></form>\
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
            "<div class=\"empty\"><b>No mail yet.</b><span>Send one with \
<code>UserMailer.welcome(user).deliver_now</code>, or preview the templates in \
<a href=\"/__soli/mailers\">Mailers</a>.</span></div>"
                .to_string()
        } else {
            format!(
                "<div class=\"empty\"><b>No message matches \u{201c}{}\u{201d}.</b>\
<span><a href=\"/__soli/inbox\">Clear the search</a></span></div>",
                dev_bar::html_escape(&needle)
            )
        });
        return html_ok(inbox_page(&body));
    }

    body.push_str(&format!(
        "<p class=\"summary\">{total} message(s){filtered} \u{b7} showing {first}\u{2013}{last} \u{b7} page {page} of {pages}</p>",
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
        "<div class=\"table-wrap\"><table><thead><tr>\
<th>Date</th><th>Status</th><th>Subject</th><th>From</th><th>To</th></tr></thead><tbody>",
    );
    for mail in &matches[start..end] {
        body.push_str(&format!(
            "<tr><td class=\"muted mono\" style=\"white-space:nowrap;\">{at}</td><td><span class=\"tag {status}\">{status}</span></td>\
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
        body.push_str(&format!("<p class=\"pager\">{}</p>", nav));
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
            "<p class=\"notice bad\">{}</p><p><a href=\"/__soli/inbox\">\u{2190} Back to the inbox</a></p>",
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
    let mut body = String::new();

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
                "<span class=\"tag failed\">failed</span> <span class=\"muted\">{}</span>",
                dev_bar::html_escape(err)
            ),
            Status::Sent => "<span class=\"tag sent\">sent</span> \
<span class=\"muted\">accepted by the SMTP server</span>"
                .to_string(),
            Status::Captured => "<span class=\"tag captured\">captured</span> \
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
    body.push_str(&format!(
        "<div class=\"table-wrap\"><table>{}</table></div>",
        rows
    ));

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
            "<div class=\"empty\" style=\"margin-top:16px;\"><b>This message has no body.</b></div>",
        );
        return html_ok(detail_page(mail, &body));
    }

    body.push_str("<div class=\"bar\" style=\"margin:16px 0 10px;\">");
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
<iframe class=\"mail\" src=\"/__soli/inbox/{}/html\" sandbox title=\"HTML body\"></iframe></div>",
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
tabs.forEach(function(t){var on=t.getAttribute('data-tab')===key;t.setAttribute('aria-pressed',on?'true':'false');\
t.style.color=on?'#e6e9ee':'';t.style.background=on?'#161a21':'';t.style.borderColor=on?'#79c0ff':'';});}\
tabs.forEach(function(t){t.addEventListener('click',function(){show(t.getAttribute('data-tab'));});});\
if(tabs.length)show(tabs[0].getAttribute('data-tab'));})();</script>",
    );

    html_ok(detail_page(mail, &body))
}

/// A message's own page: its subject as the heading, the way back under it.
fn detail_page(mail: &CapturedMail, body: &str) -> String {
    let subject = if mail.subject.trim().is_empty() {
        "(no subject)"
    } else {
        mail.subject.as_str()
    };
    operator_shell::page(
        Section::Inbox,
        subject,
        "<a href=\"/__soli/inbox\">\u{2190} Inbox</a>",
        body,
    )
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
