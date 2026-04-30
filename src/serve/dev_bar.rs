//! Dev bar overlay — auto-injected into HTML responses when running `--dev`.
//!
//! Mirrors the live-reload injection pattern in `live_reload.rs`: when the
//! response is `text/html` and dev mode is on, we splice a self-contained
//! `<aside>` + inline script before the closing `</body>` tag. The bar shows
//! method/path, status, render time, request counter, RSS, AQL query count
//! and durations (with bind-vars inlined), and a server clock.
//!
//! The rendered HTML is fully self-contained — no external CSS/JS — so it
//! works on every Soli project without any template change.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use crate::interpreter::builtins::http_log::LoggedHttpRequest;
use crate::interpreter::builtins::model::query_log::LoggedQuery;
use crate::serve::live_reload::rfind_ascii_case_insensitive;

/// Per-process request counter. Starts at 1 on the first injection.
static REQ_COUNT: AtomicU64 = AtomicU64::new(0);

/// Marker injected so we never double-inject (e.g. nested layouts).
const MARKER: &str = "__solidev_bar_injected";

/// Data the response thread captures and hands to the injector.
pub struct DevBarContext<'a> {
    pub method: &'a str,
    pub path: &'a str,
    pub status: u16,
    pub started: Instant,
    pub queries: Vec<LoggedQuery>,
    pub http_requests: Vec<LoggedHttpRequest>,
    /// Per-phase wall-clock microseconds, e.g. `("middleware", 1234)`,
    /// `("view", 9876)`. "controller" is computed from the rest.
    pub phases: Vec<(String, u64)>,
}

/// Inject the dev bar into an HTML body. Idempotent: returns input unchanged
/// if the marker is already present.
pub fn inject_dev_bar(html: &str, ctx: &DevBarContext<'_>) -> String {
    if html.contains(MARKER) {
        return html.to_string();
    }

    let bar = render_bar(ctx);

    if let Some(pos) = rfind_ascii_case_insensitive(html, b"</body>") {
        let mut out = String::with_capacity(html.len() + bar.len());
        out.push_str(&html[..pos]);
        out.push_str(&bar);
        out.push_str(&html[pos..]);
        out
    } else if let Some(pos) = rfind_ascii_case_insensitive(html, b"</html>") {
        let mut out = String::with_capacity(html.len() + bar.len());
        out.push_str(&html[..pos]);
        out.push_str(&bar);
        out.push_str(&html[pos..]);
        out
    } else {
        format!("{}{}", html, bar)
    }
}

fn render_bar(ctx: &DevBarContext<'_>) -> String {
    let elapsed_us = ctx.started.elapsed().as_micros() as u64;
    let render_str = fmt_duration_us(elapsed_us);
    let req_n = REQ_COUNT.fetch_add(1, Ordering::Relaxed) + 1;
    let rss_str = read_rss_str();
    let env_str = std::env::var("APP_ENV").unwrap_or_else(|_| "development".to_string());

    let q_count = ctx.queries.len();
    let q_total_us: u64 = ctx
        .queries
        .iter()
        .map(|q| (q.duration_ms * 1000.0).max(0.0) as u64)
        .sum();
    let q_total_str = fmt_duration_us(q_total_us);

    let status = ctx.status;
    let status_color = match status {
        100..=199 => "#8be9fd",
        200..=299 => "#b8e986",
        300..=399 => "#f0c674",
        400..=499 => "#ffb86c",
        _ => "#ff6b6b",
    };

    let clock_str = current_clock_str();

    // Phase breakdown. middleware/view come from phase_log; db/http reuse the
    // existing per-call totals; controller is whatever is left over.
    let mw_us: u64 = ctx
        .phases
        .iter()
        .filter(|(k, _)| k == "middleware")
        .map(|(_, v)| *v)
        .sum();
    let view_us: u64 = ctx
        .phases
        .iter()
        .filter(|(k, _)| k == "view")
        .map(|(_, v)| *v)
        .sum();
    let h_us_total: u64 = ctx
        .http_requests
        .iter()
        .map(|r| (r.duration_ms * 1000.0).max(0.0) as u64)
        .sum();
    let measured_us = mw_us
        .saturating_add(view_us)
        .saturating_add(q_total_us)
        .saturating_add(h_us_total);
    let controller_us = elapsed_us.saturating_sub(measured_us);
    let breakdown_panel = format!(
        "<div id=\"__solidev_phases\" style=\"display:none;border-top:1px solid #30363d;background:#08090b;padding:0.5rem 0.75rem;\">\
<div style=\"margin-bottom:0.5rem;font-size:10px;color:#8b949e;letter-spacing:0.08em;\">RENDER · {total}</div>\
<ol style=\"list-style:none;margin:0;padding:0;display:flex;flex-direction:column;gap:0.25rem;font-size:11px;\">\
{mw_row}\
{ctrl_row}\
{view_row}\
{db_row}\
{http_row}\
</ol>\
</div>",
        total = html_escape(&render_str),
        mw_row = phase_row("middleware", mw_us, elapsed_us, "#f0c674"),
        ctrl_row = phase_row("controller", controller_us, elapsed_us, "#8be9fd"),
        view_row = phase_row("view", view_us, elapsed_us, "#b8e986"),
        db_row = phase_row("db", q_total_us, elapsed_us, "#bd93f9"),
        http_row = phase_row("http", h_us_total, elapsed_us, "#ff79c6"),
    );

    // Group queries by template (the raw `query` string with @bind placeholders
    // intact) so we can flag N+1: the same template fired ≥2 times in a single
    // request is almost always a loop that should be batched (HABTM lookups
    // start at 2 — one per parent — so a stricter threshold misses them).
    let n1_groups = detect_n_plus_one(&ctx.queries, 2);
    let has_n1 = !n1_groups.is_empty();

    let queries_panel = if q_count == 0 {
        String::new()
    } else {
        let mut rows = String::new();
        for q in &ctx.queries {
            let dur_us = (q.duration_ms * 1000.0).max(0.0) as u64;
            let dur = fmt_duration_us(dur_us);
            let text = embed_binds(&q.query, q.bind_vars.as_ref());
            rows.push_str(&format!(
                "<li style=\"display:flex;align-items:flex-start;gap:0.75rem;\">\
<span style=\"flex:0 0 auto;color:#b8e986;width:5rem;text-align:right;font-variant-numeric:tabular-nums;\">{}</span>\
<pre style=\"flex:1;white-space:pre-wrap;word-break:break-all;margin:0;color:#e6e6e6;font-family:inherit;cursor:text;user-select:all;\">{}</pre>\
</li>",
                html_escape(&dur),
                html_escape(&text),
            ));
        }

        // N+1 alert block, rendered above the per-query list when at least one
        // template fired ≥3 times. Template is shown verbatim (with @binds) so
        // the user can grep for it; suggestion is to batch with `IN [...]`.
        let n1_block = if has_n1 {
            let mut alerts = String::new();
            for (template, count, total_us) in &n1_groups {
                alerts.push_str(&format!(
                    "<li style=\"display:flex;align-items:flex-start;gap:0.75rem;\">\
<span style=\"flex:0 0 auto;color:#ff6b6b;width:5rem;text-align:right;font-variant-numeric:tabular-nums;\">×{count}</span>\
<div style=\"flex:1;\">\
<pre style=\"white-space:pre-wrap;word-break:break-all;margin:0 0 0.25rem 0;color:#e6e6e6;font-family:inherit;cursor:text;user-select:all;\">{tmpl}</pre>\
<span style=\"font-size:10px;color:#8b949e;\">total {total} · likely fired in a loop — batch with <span style=\"color:#f0c674;\">FILTER doc.field IN @ids</span></span>\
</div>\
</li>",
                    count = count,
                    tmpl = html_escape(template),
                    total = html_escape(&fmt_duration_us(*total_us)),
                ));
            }
            format!(
                "<div style=\"margin-bottom:0.5rem;padding:0.5rem 0.625rem;border-left:3px solid #ff6b6b;background:#2a0d0f;border-radius:0 0.25rem 0.25rem 0;\">\
<div style=\"color:#ff6b6b;font-size:10px;letter-spacing:0.08em;margin-bottom:0.375rem;\">⚠ N+1 DETECTED · {} TEMPLATE{}</div>\
<ol style=\"list-style:none;margin:0;padding:0;display:flex;flex-direction:column;gap:0.5rem;font-size:11px;\">{}</ol>\
</div>",
                n1_groups.len(),
                if n1_groups.len() == 1 { "" } else { "S" },
                alerts,
            )
        } else {
            String::new()
        };

        let plural = if q_count == 1 { "Y" } else { "IES" };
        format!(
            "<div id=\"__solidev_queries\" style=\"display:none;border-top:1px solid #30363d;background:#08090b;max-height:40vh;overflow-y:auto;padding:0.5rem 0.75rem;\">\
<div style=\"margin-bottom:0.5rem;font-size:10px;color:#8b949e;letter-spacing:0.08em;\">SOLIDB · {} QUER{} · {}</div>\
{n1_block}\
<ol style=\"list-style:none;margin:0;padding:0;display:flex;flex-direction:column;gap:0.375rem;font-size:11px;\">{}</ol>\
</div>",
            q_count,
            plural,
            html_escape(&q_total_str),
            rows,
            n1_block = n1_block,
        )
    };

    let db_label_color = if has_n1 { "#ff6b6b" } else { "#c9d1d9" };
    let n1_warn_glyph = if has_n1 {
        " <span style=\"color:#ff6b6b;\" title=\"N+1 detected\">⚠</span>"
    } else {
        ""
    };

    let q_btn_extra = if q_count > 0 {
        format!(
            "<span style=\"color:#8b949e;\"> · </span><span style=\"color:#b8e986;\">{}</span>{}",
            html_escape(&q_total_str),
            n1_warn_glyph,
        )
    } else {
        String::new()
    };

    let h_count = ctx.http_requests.len();
    let h_total_us: u64 = ctx
        .http_requests
        .iter()
        .map(|r| (r.duration_ms * 1000.0).max(0.0) as u64)
        .sum();
    let h_total_str = fmt_duration_us(h_total_us);

    let http_panel = if h_count == 0 {
        String::new()
    } else {
        let mut rows = String::new();
        for r in &ctx.http_requests {
            let dur_us = (r.duration_ms * 1000.0).max(0.0) as u64;
            let dur = fmt_duration_us(dur_us);
            let (status_label, status_color) = if let Some(err) = &r.error {
                (format!("ERR: {}", err), "#ff6b6b")
            } else {
                let color = match r.status {
                    100..=199 => "#8be9fd",
                    200..=299 => "#b8e986",
                    300..=399 => "#f0c674",
                    400..=499 => "#ffb86c",
                    _ => "#ff6b6b",
                };
                (r.status.to_string(), color)
            };
            rows.push_str(&format!(
                "<li style=\"display:flex;align-items:flex-start;gap:0.75rem;\">\
<span style=\"flex:0 0 auto;color:#b8e986;width:5rem;text-align:right;font-variant-numeric:tabular-nums;\">{dur}</span>\
<span style=\"flex:0 0 auto;color:{sc};width:3.5rem;font-variant-numeric:tabular-nums;\">{slabel}</span>\
<span style=\"flex:0 0 auto;color:#8be9fd;width:3.5rem;\">{method}</span>\
<span style=\"flex:1;color:#e6e6e6;word-break:break-all;user-select:all;\">{url}</span>\
</li>",
                dur = html_escape(&dur),
                sc = status_color,
                slabel = html_escape(&status_label),
                method = html_escape(&r.method),
                url = html_escape(&r.url),
            ));
        }
        let plural = if h_count == 1 { "" } else { "S" };
        format!(
            "<div id=\"__solidev_http\" style=\"display:none;border-top:1px solid #30363d;background:#08090b;max-height:40vh;overflow-y:auto;padding:0.5rem 0.75rem;\">\
<div style=\"margin-bottom:0.5rem;font-size:10px;color:#8b949e;letter-spacing:0.08em;\">HTTP · {} REQUEST{} · {}</div>\
<ol style=\"list-style:none;margin:0;padding:0;display:flex;flex-direction:column;gap:0.375rem;font-size:11px;\">{}</ol>\
</div>",
            h_count,
            plural,
            html_escape(&h_total_str),
            rows,
        )
    };

    let h_btn_extra = if h_count > 0 {
        format!(
            "<span style=\"color:#8b949e;\"> · </span><span style=\"color:#b8e986;\">{}</span>",
            html_escape(&h_total_str)
        )
    } else {
        String::new()
    };

    format!(
        "<!-- {marker} -->\
<aside id=\"__solidev_bar\" style=\"position:fixed;bottom:0;left:0;right:0;z-index:2147483646;font-family:'JetBrains Mono',ui-monospace,SFMono-Regular,Menlo,Consolas,monospace;font-size:12px;background:#0b0d0f;color:#c9d1d9;border-top:1px solid #30363d;\">\
<div style=\"display:flex;align-items:center;gap:0.75rem;padding:0.375rem 0.75rem;overflow-x:auto;white-space:nowrap;\">\
<span style=\"padding:0 0.375rem;border-radius:0.25rem;background:#3a2a00;color:#f0c674;\" title=\"APP_ENV\">DEV · {env}</span>\
<span style=\"color:#30363d;\">|</span>\
<span title=\"HTTP method · path\"><span style=\"color:#8be9fd;\">{method}</span> <span style=\"color:#e6e6e6;\">{path}</span></span>\
<span style=\"color:#30363d;\">|</span>\
<span title=\"response status\">status <span style=\"color:{status_color};\">{status}</span></span>\
<span style=\"color:#30363d;\">|</span>\
<button type=\"button\" id=\"__solidev_rb\" title=\"click to expand render breakdown (middleware / controller / view / db / http)\" style=\"padding:0 0.25rem;border-radius:0.25rem;color:#c9d1d9;font:inherit;cursor:pointer;border:none;background:transparent;\">render <span style=\"color:#b8e986;\">{render}</span></button>\
<span style=\"color:#30363d;\">|</span>\
<span title=\"requests served by this worker since boot\">req <span style=\"color:#b8e986;\">#{req_n}</span></span>\
<span style=\"color:#30363d;\">|</span>\
<span title=\"resident memory of this worker\">rss <span style=\"color:#b8e986;\">{rss}</span></span>\
<span style=\"color:#30363d;\">|</span>\
<button type=\"button\" id=\"__solidev_db\" title=\"click to expand SolidB queries for this request\" style=\"padding:0 0.25rem;border-radius:0.25rem;color:{db_label_color};font:inherit;cursor:pointer;border:none;background:transparent;\">db <span style=\"color:#b8e986;\">{q_count}q</span>{q_btn_extra}</button>\
<span style=\"color:#30363d;\">|</span>\
<button type=\"button\" id=\"__solidev_hb\" title=\"click to expand outgoing HTTP requests for this request\" style=\"padding:0 0.25rem;border-radius:0.25rem;color:#c9d1d9;font:inherit;cursor:pointer;border:none;background:transparent;\">http <span style=\"color:#b8e986;\">{h_count}r</span>{h_btn_extra}</button>\
<span style=\"color:#30363d;\">|</span>\
<span style=\"color:#8b949e;\" title=\"server clock\">{clock}</span>\
<button type=\"button\" id=\"__solidev_close\" aria-label=\"Hide dev bar (Alt+D)\" title=\"hide (Alt+D)\" style=\"margin-left:auto;padding:0 0.5rem;border-radius:0.25rem;color:#c9d1d9;font:inherit;cursor:pointer;border:none;background:transparent;\">×</button>\
</div>{breakdown_panel}{queries_panel}{http_panel}</aside>\
<button type=\"button\" id=\"__solidev_show\" aria-label=\"Show dev bar\" style=\"display:none;position:fixed;bottom:0.5rem;right:0.5rem;z-index:2147483646;font-family:'JetBrains Mono',ui-monospace,monospace;font-size:10px;padding:0.25rem 0.5rem;border-radius:0.25rem;background:#0b0d0f;color:#f0c674;border:1px solid #30363d;letter-spacing:0.05em;cursor:pointer;\">DEV</button>\
<script>(function(){{var bar=document.getElementById('__solidev_bar');var open=document.getElementById('__solidev_show');if(!bar||!open)return;function setHidden(h){{if(h){{bar.style.display='none';open.style.display='inline-flex';try{{sessionStorage.setItem('__solidev_hidden','1');}}catch(e){{}}}}else{{bar.style.display='';open.style.display='none';try{{sessionStorage.removeItem('__solidev_hidden');}}catch(e){{}}}}}}var hidden=false;try{{hidden=sessionStorage.getItem('__solidev_hidden')==='1';}}catch(e){{}}setHidden(hidden);var c=document.getElementById('__solidev_close');if(c)c.addEventListener('click',function(){{setHidden(true);}});open.addEventListener('click',function(){{setHidden(false);}});var db=document.getElementById('__solidev_db');var qp=document.getElementById('__solidev_queries');if(db&&qp){{db.addEventListener('click',function(){{qp.style.display=qp.style.display==='none'?'block':'none';}});}}var hb=document.getElementById('__solidev_hb');var hp=document.getElementById('__solidev_http');if(hb&&hp){{hb.addEventListener('click',function(){{hp.style.display=hp.style.display==='none'?'block':'none';}});}}var rb=document.getElementById('__solidev_rb');var rp=document.getElementById('__solidev_phases');if(rb&&rp){{rb.addEventListener('click',function(){{rp.style.display=rp.style.display==='none'?'block':'none';}});}}document.addEventListener('keydown',function(e){{if(e.altKey&&(e.key==='d'||e.key==='D')){{e.preventDefault();setHidden(bar.style.display!=='none');}}}});}})();</script>",
        marker = MARKER,
        env = html_escape(&env_str),
        method = html_escape(ctx.method),
        path = html_escape(ctx.path),
        status_color = status_color,
        status = status,
        render = html_escape(&render_str),
        req_n = req_n,
        rss = html_escape(&rss_str),
        q_count = q_count,
        q_btn_extra = q_btn_extra,
        db_label_color = db_label_color,
        h_count = h_count,
        h_btn_extra = h_btn_extra,
        clock = html_escape(&clock_str),
        queries_panel = queries_panel,
        http_panel = http_panel,
        breakdown_panel = breakdown_panel,
    )
}

/// Render one row of the render-breakdown panel: phase name, duration, % bar.
fn phase_row(name: &str, us: u64, total_us: u64, color: &str) -> String {
    let pct = if total_us == 0 {
        0
    } else {
        ((us as f64 / total_us as f64) * 100.0).round() as u32
    };
    let bar_width_pct = pct.min(100);
    format!(
        "<li style=\"display:flex;align-items:center;gap:0.75rem;\">\
<span style=\"flex:0 0 5.5rem;color:{color};\">{name}</span>\
<span style=\"flex:0 0 4.5rem;color:#e6e6e6;font-variant-numeric:tabular-nums;text-align:right;\">{dur}</span>\
<span style=\"flex:0 0 2.5rem;color:#8b949e;font-variant-numeric:tabular-nums;text-align:right;\">{pct}%</span>\
<span style=\"flex:1;height:0.5rem;background:#1c1f23;border-radius:0.125rem;overflow:hidden;\"><span style=\"display:block;width:{bar}%;height:100%;background:{color};\"></span></span>\
</li>",
        color = color,
        name = html_escape(name),
        dur = html_escape(&fmt_duration_us(us)),
        pct = pct,
        bar = bar_width_pct,
    )
}

/// Group queries by their raw template (the AQL string before bind-substitution).
/// Returns groups with count >= `threshold`, sorted by count desc.
///
/// The template is the natural fingerprint for an N+1: only the bind values
/// differ between repeated calls, so the `query` field is identical across
/// every iteration of the offending loop.
fn detect_n_plus_one(queries: &[LoggedQuery], threshold: usize) -> Vec<(String, usize, u64)> {
    use std::collections::HashMap;
    let mut groups: HashMap<&str, (usize, u64)> = HashMap::new();
    for q in queries {
        let dur_us = (q.duration_ms * 1000.0).max(0.0) as u64;
        let entry = groups.entry(q.query.as_str()).or_insert((0, 0));
        entry.0 += 1;
        entry.1 += dur_us;
    }
    let mut result: Vec<(String, usize, u64)> = groups
        .into_iter()
        .filter(|(_, (count, _))| *count >= threshold)
        .map(|(template, (count, total))| (template.to_string(), count, total))
        .collect();
    result.sort_by(|a, b| b.1.cmp(&a.1).then(b.2.cmp(&a.2)));
    result
}

fn fmt_duration_us(us: u64) -> String {
    if us < 1_000 {
        format!("{}µs", us)
    } else if us < 1_000_000 {
        let whole = us / 1_000;
        let tenth = (us - whole * 1_000) / 100;
        format!("{}.{}ms", whole, tenth)
    } else {
        let whole = us / 1_000_000;
        let tenth = (us - whole * 1_000_000) / 100_000;
        format!("{}.{}s", whole, tenth)
    }
}

fn read_rss_str() -> String {
    // Linux-only; on other platforms the file won't exist and we return "?".
    let status = match std::fs::read_to_string("/proc/self/status") {
        Ok(s) => s,
        Err(_) => return "?".to_string(),
    };
    let kb: u64 = status
        .lines()
        .find_map(|line| line.strip_prefix("VmRSS:"))
        .and_then(|rest| rest.split_whitespace().next())
        .and_then(|n| n.parse().ok())
        .unwrap_or(0);
    if kb < 1024 {
        format!("{}kB", kb)
    } else {
        let mb_whole = kb / 1024;
        let mb_tenth = ((kb - mb_whole * 1024) * 10) / 1024;
        format!("{}.{}MB", mb_whole, mb_tenth)
    }
}

fn current_clock_str() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let day_secs = secs % 86_400;
    let h = day_secs / 3600;
    let m = (day_secs % 3600) / 60;
    let s = day_secs % 60;
    format!("{:02}:{:02}:{:02} UTC", h, m, s)
}

fn embed_binds(
    query: &str,
    binds: Option<&std::collections::HashMap<String, serde_json::Value>>,
) -> String {
    let Some(map) = binds else {
        return query.to_string();
    };
    // Substitute longest keys first so `@ab` isn't shadowed by `@a`.
    let mut keys: Vec<&String> = map.keys().collect();
    keys.sort_by(|a, b| b.len().cmp(&a.len()));
    let mut result = query.to_string();
    for k in keys {
        let v = &map[k];
        let repl = if k.starts_with('@') {
            // `@@coll` — splice as bare identifier (collection names are strings in AQL).
            match v {
                serde_json::Value::String(s) => s.clone(),
                other => other.to_string(),
            }
        } else {
            v.to_string()
        };
        result = result.replace(&format!("@{}", k), &repl);
    }
    result
}

fn html_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(ch),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx<'a>(method: &'a str, path: &'a str) -> DevBarContext<'a> {
        DevBarContext {
            method,
            path,
            status: 200,
            started: Instant::now(),
            queries: vec![],
            http_requests: vec![],
            phases: vec![],
        }
    }

    #[test]
    fn injects_before_body() {
        let html = "<html><body><h1>hi</h1></body></html>";
        let out = inject_dev_bar(html, &ctx("GET", "/"));
        assert!(out.contains(MARKER));
        assert!(out.contains("__solidev_bar"));
        let bar_pos = out.find("__solidev_bar").unwrap();
        let body_pos = out.find("</body>").unwrap();
        assert!(bar_pos < body_pos);
    }

    #[test]
    fn idempotent() {
        let html = "<html><body></body></html>";
        let once = inject_dev_bar(html, &ctx("GET", "/"));
        let twice = inject_dev_bar(&once, &ctx("GET", "/"));
        assert_eq!(once.matches("__solidev_bar\"").count(), 1);
        assert_eq!(twice.matches("__solidev_bar\"").count(), 1);
    }

    #[test]
    fn html_escapes_path() {
        let html = "<html><body></body></html>";
        let out = inject_dev_bar(html, &ctx("GET", "/x?a=<script>"));
        assert!(out.contains("&lt;script&gt;"));
        assert!(!out.contains("/x?a=<script>"));
    }

    #[test]
    fn fmt_duration_buckets() {
        assert_eq!(fmt_duration_us(500), "500µs");
        assert_eq!(fmt_duration_us(1_500), "1.5ms");
        assert_eq!(fmt_duration_us(2_500_000), "2.5s");
    }

    #[test]
    fn embed_binds_replaces_keys() {
        let mut binds = std::collections::HashMap::new();
        binds.insert("name".to_string(), serde_json::json!("Alice"));
        let out = embed_binds("FOR u IN users FILTER u.name == @name RETURN u", Some(&binds));
        assert!(out.contains("\"Alice\""));
        assert!(!out.contains("@name"));
    }

    fn q(template: &str, dur_ms: f64) -> LoggedQuery {
        LoggedQuery {
            query: template.into(),
            bind_vars: None,
            duration_ms: dur_ms,
        }
    }

    #[test]
    fn detect_n_plus_one_groups_by_template() {
        let queries = vec![
            q("FOR doc IN users FILTER doc._key == @key RETURN doc", 0.5),
            q("FOR doc IN order_items FILTER doc.order_id == @id RETURN doc", 0.1),
            q("FOR doc IN order_items FILTER doc.order_id == @id RETURN doc", 0.1),
            q("FOR doc IN order_items FILTER doc.order_id == @id RETURN doc", 0.1),
        ];
        let groups = detect_n_plus_one(&queries, 3);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].1, 3);
        assert!(groups[0].0.contains("order_items"));
    }

    #[test]
    fn detect_n_plus_one_respects_threshold() {
        let queries = vec![q("FOR x IN y RETURN x", 0.1), q("FOR x IN y RETURN x", 0.1)];
        // 2 calls under threshold 3 → no alert
        assert!(detect_n_plus_one(&queries, 3).is_empty());
        // Same data flagged at threshold 2
        assert_eq!(detect_n_plus_one(&queries, 2).len(), 1);
    }

    #[test]
    fn renders_n_plus_one_warning() {
        let html = "<html><body></body></html>";
        let mut c = ctx("GET", "/orders");
        for _ in 0..14 {
            c.queries.push(q(
                "FOR doc IN order_items FILTER doc.order_id == @id RETURN doc",
                0.1,
            ));
        }
        let out = inject_dev_bar(html, &c);
        assert!(out.contains("N+1 DETECTED"));
        assert!(out.contains("×14"));
        // db badge should switch to red
        assert!(out.contains("color:#ff6b6b") && out.contains("N+1 detected"));
    }

    #[test]
    fn renders_http_panel_with_request() {
        let html = "<html><body></body></html>";
        let mut c = ctx("GET", "/");
        c.http_requests.push(LoggedHttpRequest {
            method: "POST".into(),
            url: "https://api.example.com/orders".into(),
            status: 201,
            duration_ms: 42.5,
            error: None,
        });
        let out = inject_dev_bar(html, &c);
        assert!(out.contains("__solidev_http"));
        assert!(out.contains("https://api.example.com/orders"));
        assert!(out.contains(">POST<"));
        assert!(out.contains(">201<"));
        assert!(out.contains("http <span"));
        assert!(out.contains("1r</span>"));
    }

    #[test]
    fn renders_breakdown_panel() {
        let html = "<html><body></body></html>";
        let mut c = ctx("GET", "/");
        c.phases.push(("middleware".into(), 2_000));
        c.phases.push(("view".into(), 5_000));
        let out = inject_dev_bar(html, &c);
        assert!(out.contains("__solidev_phases"));
        // Each phase row is rendered, including the derived "controller".
        assert!(out.contains(">middleware<"));
        assert!(out.contains(">view<"));
        assert!(out.contains(">controller<"));
        assert!(out.contains(">db<"));
        assert!(out.contains(">http<"));
    }

    #[test]
    fn phase_row_handles_zero_total() {
        // When elapsed is 0, the percentage should be 0 not NaN/panic.
        let row = phase_row("view", 0, 0, "#fff");
        assert!(row.contains("0%"));
    }

    #[test]
    fn renders_http_error() {
        let html = "<html><body></body></html>";
        let mut c = ctx("GET", "/");
        c.http_requests.push(LoggedHttpRequest {
            method: "GET".into(),
            url: "https://down.example.com/".into(),
            status: 0,
            duration_ms: 5.0,
            error: Some("dns failure".into()),
        });
        let out = inject_dev_bar(html, &c);
        assert!(out.contains("ERR: dns failure"));
    }

    #[test]
    fn embed_binds_longest_first() {
        let mut binds = std::collections::HashMap::new();
        binds.insert("a".to_string(), serde_json::json!(1));
        binds.insert("ab".to_string(), serde_json::json!(2));
        let out = embed_binds("@ab + @a", Some(&binds));
        assert_eq!(out, "2 + 1");
    }
}
