//! Dev bar overlay — auto-injected into HTML responses when running `--dev`.
//!
//! Mirrors the live-reload injection pattern in `live_reload.rs`: when the
//! response is `text/html` and dev mode is on, we splice a self-contained
//! `<aside>` + inline script before the closing `</body>` tag. The bar shows
//! method/path, status, render time, RSS, AQL query count and durations
//! (with bind-vars inlined), and a server clock.
//!
//! The rendered HTML is fully self-contained — no external CSS/JS — so it
//! works on every Soli project without any template change.

use std::time::Instant;

use crate::interpreter::builtins::http_log::LoggedHttpRequest;
use crate::interpreter::builtins::model::query_log::LoggedQuery;
use crate::serve::live_reload::rfind_ascii_case_insensitive;
use crate::serve::span_log::{SpanKind, SpanRecord};

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
    /// One entry per middleware call in the order they fired
    /// (`(name, dur_us)`). When more than one middleware ran on this
    /// request, the render-breakdown expands the aggregate "middleware"
    /// row into per-middleware sub-rows.
    pub middlewares: Vec<(String, u64)>,
    /// One entry per template render (top-level view, layout, partial)
    /// in the order they fired (`(id, name, dur_us)`). The render-breakdown
    /// expands the aggregate "view" row into per-template sub-rows so
    /// the user can see exactly which templates ran. Durations include
    /// nested children, so they overlap and don't sum to the aggregate.
    /// `id` is a stable per-request render id assigned at render *start*;
    /// the template engine wraps the rendered output in
    /// `<!--solidev:KIND:start id=ID …-->` markers using the same id, and
    /// the dev bar emits it as `data-solidev-view-idx` on the sub-row so
    /// the hover-overlay JS can pair them.
    /// `parent` is the id of the lexically-enclosing render (e.g. a
    /// partial's parent is the view that included it; the view's parent
    /// is its layout if any). The dev bar uses this to indent each
    /// sub-row by depth so the breakdown reads as a tree.
    pub views: Vec<(u32, Option<u32>, String, u64)>,
    /// Hierarchical spans for the flamegraph panel. Empty in non-dev
    /// mode. Each span carries (id, parent, name, kind, start_us,
    /// end_us, meta).
    pub spans: Vec<SpanRecord>,
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
    let mw_label = if ctx.middlewares.len() > 1 {
        format!("middleware ({})", ctx.middlewares.len())
    } else {
        "middleware".to_string()
    };
    let mw_row = if ctx.middlewares.is_empty() {
        phase_row(&mw_label, mw_us, elapsed_us, "#f0c674")
    } else {
        aggregate_row_clickable(
            "__solidev_mw_toggle",
            "__solidev_mw_chev",
            "click to toggle per-middleware breakdown",
            &mw_label,
            mw_us,
            elapsed_us,
            "#f0c674",
        )
    };
    let mw_sub_rows = if ctx.middlewares.is_empty() {
        String::new()
    } else {
        let mut s = String::new();
        for (name, us) in &ctx.middlewares {
            s.push_str(&sub_row(name, *us, elapsed_us, "#f0c674"));
        }
        format!(
            "<li id=\"__solidev_mw_subrows\" style=\"display:none;list-style:none;padding:0;margin:0;\">\
<ul style=\"list-style:none;margin:0;padding:0;display:flex;flex-direction:column;gap:0.25rem;\">\
{}\
</ul>\
</li>",
            s
        )
    };

    let view_label = if ctx.views.len() > 1 {
        format!("view ({})", ctx.views.len())
    } else {
        "view".to_string()
    };
    let view_row = if ctx.views.is_empty() {
        phase_row(&view_label, view_us, elapsed_us, "#b8e986")
    } else {
        aggregate_row_clickable(
            "__solidev_view_toggle",
            "__solidev_view_chev",
            "click to toggle per-template breakdown (views, layouts, partials)",
            &view_label,
            view_us,
            elapsed_us,
            "#b8e986",
        )
    };
    let view_sub_rows = if ctx.views.is_empty() {
        String::new()
    } else {
        // Render the view list as a tree: walk parent → children, depth
        // controls indentation. `views` is in close-order (children
        // before parents), so we build a child-list map then emit a
        // pre-order DFS starting at every root (entries whose parent is
        // None or whose parent isn't in this snapshot).
        let id_set: std::collections::HashSet<u32> =
            ctx.views.iter().map(|(id, _, _, _)| *id).collect();
        let mut children_of: std::collections::HashMap<Option<u32>, Vec<u32>> =
            std::collections::HashMap::new();
        let mut entry_by_id: std::collections::HashMap<u32, &(u32, Option<u32>, String, u64)> =
            std::collections::HashMap::new();
        let mut roots: Vec<u32> = Vec::new();
        for entry in &ctx.views {
            let (id, parent, _, _) = entry;
            entry_by_id.insert(*id, entry);
            let parent_in_snapshot = parent.filter(|p| id_set.contains(p));
            if parent_in_snapshot.is_none() {
                roots.push(*id);
            }
            children_of.entry(parent_in_snapshot).or_default().push(*id);
        }
        // Siblings render sequentially (one partial finishes before the
        // next starts), so close-order *within* a sibling group already
        // matches start-order — no reversal needed. Children appear
        // before their parent in `ctx.views` because the parent closes
        // last, but `children_of` only collects siblings under the same
        // parent, so each list is naturally start-ordered.

        fn emit(
            id: u32,
            depth: u32,
            elapsed_us: u64,
            entry_by_id: &std::collections::HashMap<u32, &(u32, Option<u32>, String, u64)>,
            children_of: &std::collections::HashMap<Option<u32>, Vec<u32>>,
            out: &mut String,
        ) {
            if let Some((rid, _, name, us)) = entry_by_id.get(&id).copied() {
                out.push_str(&view_sub_row(*rid, depth, name, *us, elapsed_us, "#b8e986"));
            }
            if let Some(kids) = children_of.get(&Some(id)) {
                for &child in kids {
                    emit(child, depth + 1, elapsed_us, entry_by_id, children_of, out);
                }
            }
        }

        let mut s = String::new();
        for &root in &roots {
            emit(root, 0, elapsed_us, &entry_by_id, &children_of, &mut s);
        }
        format!(
            "<li id=\"__solidev_view_subrows\" style=\"display:none;list-style:none;padding:0;margin:0;\">\
<ul style=\"list-style:none;margin:0;padding:0;display:flex;flex-direction:column;gap:0.25rem;\">\
{}\
</ul>\
</li>",
            s
        )
    };
    let breakdown_panel = format!(
        "<div id=\"__solidev_phases\" style=\"display:none;border-top:1px solid #30363d;background:#08090b;padding:0.5rem 0.75rem;max-height:33vh;overflow-y:auto;\">\
<div style=\"margin-bottom:0.5rem;font-size:10px;color:#8b949e;letter-spacing:0.08em;\">RENDER · {total}</div>\
<ol style=\"list-style:none;margin:0;padding:0;display:flex;flex-direction:column;gap:0.25rem;font-size:11px;\">\
{mw_row}\
{mw_sub_rows}\
{ctrl_row}\
{view_row}\
{view_sub_rows}\
{db_row}\
{http_row}\
</ol>\
</div>",
        total = html_escape(&render_str),
        mw_row = mw_row,
        mw_sub_rows = mw_sub_rows,
        ctrl_row = phase_row("controller", controller_us, elapsed_us, "#8be9fd"),
        view_row = view_row,
        view_sub_rows = view_sub_rows,
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

    // Flamegraph panel: hierarchical view of every captured span. Empty
    // when the request didn't open any spans (e.g. dev mode off, or a
    // 404 with no controller dispatch).
    let flame_count = ctx.spans.len();
    let flame_panel = render_flame_panel(&ctx.spans, elapsed_us);

    format!(
        "<!-- {marker} -->\
<aside id=\"__solidev_bar\" style=\"position:fixed;bottom:0;left:0;right:0;z-index:2147483646;font-family:'JetBrains Mono',ui-monospace,SFMono-Regular,Menlo,Consolas,monospace;font-size:12px;background:#0b0d0f;color:#c9d1d9;border-top:1px solid #30363d;max-height:100vh;overflow-y:auto;\">\
<div style=\"display:flex;align-items:center;gap:0.75rem;padding:0.375rem 0.75rem;overflow-x:auto;white-space:nowrap;position:sticky;top:0;background:#0b0d0f;z-index:1;border-bottom:1px solid #30363d;\">\
<span style=\"padding:0 0.375rem;border-radius:0.25rem;background:#3a2a00;color:#f0c674;\" title=\"APP_ENV\">DEV · {env}</span>\
<span style=\"color:#30363d;\">|</span>\
<span title=\"HTTP method · path · status\"><span style=\"color:#8be9fd;\">{method}</span> <span style=\"color:#e6e6e6;\">{path}</span> <span style=\"color:#8b949e;\">[</span><span style=\"color:{status_color};\">{status}</span><span style=\"color:#8b949e;\">]</span></span>\
<span style=\"color:#30363d;\">|</span>\
<button type=\"button\" id=\"__solidev_rb\" title=\"click to expand render breakdown (middleware / controller / view / db / http)\" style=\"padding:0 0.25rem;border-radius:0.25rem;color:#c9d1d9;font:inherit;cursor:pointer;border:none;background:transparent;\">render <span style=\"color:#b8e986;\">{render}</span></button>\
<span style=\"color:#30363d;\">|</span>\
<span title=\"resident memory of this worker\">rss <span style=\"color:#b8e986;\">{rss}</span></span>\
<span style=\"color:#30363d;\">|</span>\
<button type=\"button\" id=\"__solidev_db\" title=\"click to expand SolidB queries for this request\" style=\"padding:0 0.25rem;border-radius:0.25rem;color:{db_label_color};font:inherit;cursor:pointer;border:none;background:transparent;\">db <span style=\"color:#b8e986;\">{q_count}q</span>{q_btn_extra}</button>\
<span style=\"color:#30363d;\">|</span>\
<button type=\"button\" id=\"__solidev_hb\" title=\"click to expand outgoing HTTP requests for this request\" style=\"padding:0 0.25rem;border-radius:0.25rem;color:#c9d1d9;font:inherit;cursor:pointer;border:none;background:transparent;\">http <span style=\"color:#b8e986;\">{h_count}r</span>{h_btn_extra}</button>\
<span style=\"color:#30363d;\">|</span>\
<button type=\"button\" id=\"__solidev_fb\" title=\"click to expand the flamegraph (hierarchical timing per phase + per Soli function)\" style=\"padding:0 0.25rem;border-radius:0.25rem;color:#c9d1d9;font:inherit;cursor:pointer;border:none;background:transparent;\">flame <span style=\"color:#b8e986;\">{flame_count}s</span></button>\
<button type=\"button\" id=\"__solidev_close\" aria-label=\"Hide dev bar (Alt+D)\" title=\"hide (Alt+D)\" style=\"margin-left:auto;padding:0 0.5rem;border-radius:0.25rem;color:#c9d1d9;font:inherit;cursor:pointer;border:none;background:transparent;\">×</button>\
</div>{breakdown_panel}{queries_panel}{http_panel}{flame_panel}</aside>\
<button type=\"button\" id=\"__solidev_show\" aria-label=\"Show dev bar\" style=\"display:none;position:fixed;bottom:0.5rem;right:0.5rem;z-index:2147483646;font-family:'JetBrains Mono',ui-monospace,monospace;font-size:10px;padding:0.25rem 0.5rem;border-radius:0.25rem;background:#0b0d0f;color:#f0c674;border:1px solid #30363d;letter-spacing:0.05em;cursor:pointer;\">DEV</button>\
<script>(function(){{var bar=document.getElementById('__solidev_bar');var open=document.getElementById('__solidev_show');if(!bar||!open)return;var origPad=document.body.style.paddingBottom;function syncPad(){{if(bar.style.display==='none'){{document.body.style.paddingBottom=origPad;return;}}document.body.style.paddingBottom=bar.offsetHeight+'px';}}function setHidden(h){{if(h){{bar.style.display='none';open.style.display='inline-flex';try{{sessionStorage.setItem('__solidev_hidden','1');}}catch(e){{}}}}else{{bar.style.display='';open.style.display='none';try{{sessionStorage.removeItem('__solidev_hidden');}}catch(e){{}}}}syncPad();}}var hidden=false;try{{hidden=sessionStorage.getItem('__solidev_hidden')==='1';}}catch(e){{}}setHidden(hidden);if(typeof ResizeObserver!=='undefined'){{try{{new ResizeObserver(syncPad).observe(bar);}}catch(e){{}}}}window.addEventListener('resize',syncPad);var c=document.getElementById('__solidev_close');if(c)c.addEventListener('click',function(){{setHidden(true);}});open.addEventListener('click',function(){{setHidden(false);}});var db=document.getElementById('__solidev_db');var qp=document.getElementById('__solidev_queries');if(db&&qp){{db.addEventListener('click',function(){{qp.style.display=qp.style.display==='none'?'block':'none';}});}}var hb=document.getElementById('__solidev_hb');var hp=document.getElementById('__solidev_http');if(hb&&hp){{hb.addEventListener('click',function(){{hp.style.display=hp.style.display==='none'?'block':'none';}});}}var rb=document.getElementById('__solidev_rb');var rp=document.getElementById('__solidev_phases');if(rb&&rp){{rb.addEventListener('click',function(){{rp.style.display=rp.style.display==='none'?'block':'none';}});}}var mwt=document.getElementById('__solidev_mw_toggle');var mws=document.getElementById('__solidev_mw_subrows');var mwc=document.getElementById('__solidev_mw_chev');if(mwt&&mws){{mwt.addEventListener('click',function(){{var hidden=mws.style.display==='none';mws.style.display=hidden?'':'none';if(mwc)mwc.textContent=hidden?'▼':'▶';}});}}var vwt=document.getElementById('__solidev_view_toggle');var vws=document.getElementById('__solidev_view_subrows');var vwc=document.getElementById('__solidev_view_chev');if(vwt&&vws){{vwt.addEventListener('click',function(){{var hidden=vws.style.display==='none';vws.style.display=hidden?'':'none';if(vwc)vwc.textContent=hidden?'▼':'▶';}});}}var fb=document.getElementById('__solidev_fb');var fp=document.getElementById('__solidev_flame');if(fb&&fp){{fb.addEventListener('click',function(){{fp.style.display=fp.style.display==='none'?'block':'none';}});}}var fchart=document.getElementById('__solidev_flame_chart');var flist=document.getElementById('__solidev_flame_list');if(fchart){{var totalUs=parseFloat(fchart.getAttribute('data-total'))||1;var rects=fchart.querySelectorAll('.__solidev_rect');function applyZoom(viewStart,viewW){{rects.forEach(function(r){{var s=parseFloat(r.getAttribute('data-start'));var w=parseFloat(r.getAttribute('data-w'));var rs=s-viewStart;var re=rs+w;if(re<=0||rs>=viewW){{r.style.display='none';return;}}r.style.display='';var cs=Math.max(0,rs);var ce=Math.min(viewW,re);r.style.left=(cs/viewW*100)+'%';r.style.width=Math.max(0.001,(ce-cs)/viewW*100)+'%';}});}}function highlightRect(rect,on){{if(!rect)return;rect.style.outline=on?'2px solid #ffffff':'';rect.style.outlineOffset=on?'-2px':'';}}function highlightRow(li,on){{if(!li)return;li.style.background=on?'#1c1f23':'';if(on)li.scrollIntoView({{block:'nearest',behavior:'smooth'}});}}rects.forEach(function(r){{r.addEventListener('click',function(ev){{ev.stopPropagation();applyZoom(parseFloat(r.getAttribute('data-start')),parseFloat(r.getAttribute('data-w')));}});r.addEventListener('mouseenter',function(){{var idx=r.getAttribute('data-idx');var li=flist?flist.querySelector('li[data-idx=\"'+idx+'\"]'):null;highlightRow(li,true);highlightRect(r,true);}});r.addEventListener('mouseleave',function(){{var idx=r.getAttribute('data-idx');var li=flist?flist.querySelector('li[data-idx=\"'+idx+'\"]'):null;highlightRow(li,false);highlightRect(r,false);}});}});fchart.addEventListener('dblclick',function(){{applyZoom(0,totalUs);}});if(flist){{flist.querySelectorAll('li[data-idx]').forEach(function(li){{li.addEventListener('mouseenter',function(){{var idx=li.getAttribute('data-idx');var rect=fchart.querySelector('.__solidev_rect[data-idx=\"'+idx+'\"]');highlightRow(li,true);highlightRect(rect,true);}});li.addEventListener('mouseleave',function(){{var idx=li.getAttribute('data-idx');var rect=fchart.querySelector('.__solidev_rect[data-idx=\"'+idx+'\"]');highlightRow(li,false);highlightRect(rect,false);}});li.addEventListener('click',function(){{applyZoom(parseFloat(li.getAttribute('data-start')),parseFloat(li.getAttribute('data-w')));}});}});}}}}var vrows=document.querySelectorAll('#__solidev_bar [data-solidev-view-idx]');if(vrows.length){{var ov=null,lbl=null,markerCache=null,autoScroll=false;function ensureOverlay(){{if(ov)return;ov=document.createElement('div');ov.id='__solidev_view_outline';ov.style.cssText='position:absolute;pointer-events:none;outline:2px solid #b8e986;outline-offset:-2px;background:rgba(184,233,134,0.12);z-index:2147483645;display:none;border-radius:2px;';document.body.appendChild(ov);lbl=document.createElement('div');lbl.style.cssText='position:absolute;pointer-events:none;font-family:JetBrains Mono,ui-monospace,monospace;font-size:10px;background:#0b0d0f;color:#b8e986;border:1px solid #b8e986;padding:1px 6px;border-radius:3px;z-index:2147483645;display:none;white-space:nowrap;';document.body.appendChild(lbl);}}function buildCache(){{if(markerCache)return markerCache;markerCache={{}};var w=document.createTreeWalker(document.body,NodeFilter.SHOW_COMMENT,null);var n;while(n=w.nextNode()){{var v=n.nodeValue||'';var m=v.match(/^solidev:(view|partial|layout):(start|end) id=(\\d+)/);if(!m)continue;var id=m[3];if(!markerCache[id])markerCache[id]={{}};markerCache[id][m[2]]=n;}}return markerCache;}}function ensureVisible(rect){{var barH=(bar&&bar.style.display!=='none')?bar.offsetHeight:0;var vh=window.innerHeight||document.documentElement.clientHeight;var visBottom=vh-barH;var pad=24;var needsUp=rect.top<pad;var needsDown=rect.top>visBottom-pad||(rect.bottom>visBottom&&rect.height<visBottom-2*pad);if(!needsUp&&!needsDown)return false;autoScroll=true;var sy=window.scrollY||window.pageYOffset||0;var targetY=sy+rect.top-Math.max(80,(visBottom-rect.height)/2);if(targetY<0)targetY=0;window.scrollTo({{top:targetY,left:window.scrollX||0,behavior:'auto'}});setTimeout(function(){{autoScroll=false;}},0);return true;}}function showFor(id,name){{var pair=buildCache()[id];if(!pair||!pair.start||!pair.end)return;var range=document.createRange();try{{range.setStartAfter(pair.start);range.setEndBefore(pair.end);}}catch(e){{return;}}var rect=range.getBoundingClientRect();if(rect.width===0&&rect.height===0)return;if(ensureVisible(rect)){{rect=range.getBoundingClientRect();}}ensureOverlay();var sx=window.scrollX||window.pageXOffset||0;var sy=window.scrollY||window.pageYOffset||0;ov.style.display='block';ov.style.left=(rect.left+sx)+'px';ov.style.top=(rect.top+sy)+'px';ov.style.width=rect.width+'px';ov.style.height=rect.height+'px';lbl.textContent=name;lbl.style.display='block';lbl.style.left=(rect.left+sx)+'px';lbl.style.top=Math.max(0,rect.top+sy-18)+'px';}}function hideOv(){{if(autoScroll)return;if(ov)ov.style.display='none';if(lbl)lbl.style.display='none';}}vrows.forEach(function(li){{li.addEventListener('mouseenter',function(){{var id=li.getAttribute('data-solidev-view-idx');var n=li.getAttribute('data-solidev-view-name');if(!n){{var nameEl=li.querySelector('span[title]');n=nameEl?nameEl.textContent:'';}}showFor(id,n);}});li.addEventListener('mouseleave',hideOv);}});}}document.addEventListener('keydown',function(e){{if(e.altKey&&(e.key==='d'||e.key==='D')){{e.preventDefault();setHidden(bar.style.display!=='none');}}}});}})();</script>",
        marker = MARKER,
        env = html_escape(&env_str),
        method = html_escape(ctx.method),
        path = html_escape(ctx.path),
        status_color = status_color,
        status = status,
        render = html_escape(&render_str),
        rss = html_escape(&rss_str),
        q_count = q_count,
        q_btn_extra = q_btn_extra,
        db_label_color = db_label_color,
        h_count = h_count,
        h_btn_extra = h_btn_extra,
        flame_count = flame_count,
        queries_panel = queries_panel,
        http_panel = http_panel,
        breakdown_panel = breakdown_panel,
        flame_panel = flame_panel,
    )
}

/// Map a SpanKind to its stripe color in the flamegraph.
fn flame_color(kind: SpanKind) -> &'static str {
    match kind {
        SpanKind::Request => "#c9d1d9",
        SpanKind::Middleware => "#f0c674",
        SpanKind::BeforeAction => "#d4a017",
        SpanKind::AfterAction => "#d4a017",
        SpanKind::Action => "#8be9fd",
        SpanKind::View => "#b8e986",
        SpanKind::Partial => "#a4d97a",
        SpanKind::Db => "#bd93f9",
        SpanKind::Http => "#ff79c6",
        SpanKind::Fn => "#6c7280",
    }
}

/// Compute each span's stack depth by walking the parent chain.
/// Returns a (Vec keyed by span index in input order) of depths.
fn compute_depths(spans: &[SpanRecord]) -> Vec<u32> {
    use std::collections::HashMap;
    let id_to_idx: HashMap<u32, usize> = spans.iter().enumerate().map(|(i, s)| (s.id, i)).collect();
    let mut depths = vec![0u32; spans.len()];
    // Spans were appended in close-order — children before parents — so a
    // straight pass over `spans` may try to read a parent's depth before
    // it's set. Walk the parent chain instead.
    for (i, s) in spans.iter().enumerate() {
        let mut d = 0u32;
        let mut cur = s.parent;
        while let Some(pid) = cur {
            d += 1;
            cur = id_to_idx.get(&pid).and_then(|&pi| spans[pi].parent);
            // Safety guard against any cycle (shouldn't happen but…)
            if d > 1024 {
                break;
            }
        }
        depths[i] = d;
    }
    depths
}

/// JSON-escape a string for inclusion in trace-event field values.
fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

/// Build a Chrome Trace Event Format JSON document. Opens cleanly in
/// chrome://tracing and ui.perfetto.dev. One "X" (complete) event per
/// span; pid/tid hard-coded since this is a single-request profile.
fn build_trace_json(spans: &[SpanRecord]) -> String {
    let mut out = String::from("{\"traceEvents\":[");
    for (i, s) in spans.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        let dur = s.end_us.saturating_sub(s.start_us);
        out.push_str("{\"name\":\"");
        out.push_str(&json_escape(&s.name));
        out.push_str("\",\"cat\":\"");
        out.push_str(s.kind.as_str());
        out.push_str("\",\"ph\":\"X\",\"pid\":1,\"tid\":1,\"ts\":");
        out.push_str(&s.start_us.to_string());
        out.push_str(",\"dur\":");
        out.push_str(&dur.to_string());
        if let Some(meta) = &s.meta {
            out.push_str(",\"args\":{\"meta\":\"");
            out.push_str(&json_escape(meta));
            out.push_str("\"}");
        }
        out.push('}');
    }
    out.push_str("],\"displayTimeUnit\":\"ns\"}");
    out
}

/// Strip the current working directory prefix from a span `meta` string so
/// the flamegraph displays `app/controllers/users.sl:42` instead of the
/// absolute path. Non-path metas (AQL templates, HTTP error messages) don't
/// start with the CWD and pass through unchanged.
fn relativize_meta(meta: &str, cwd_prefix: &str) -> String {
    if !cwd_prefix.is_empty() && meta.starts_with(cwd_prefix) {
        meta[cwd_prefix.len()..].trim_start_matches('/').to_string()
    } else {
        meta.to_string()
    }
}

/// Render the inline-SVG flamegraph panel + trace JSON download link.
/// Returns the empty string when `spans` is empty so the closing `</aside>`
/// stays valid.
fn render_flame_panel(spans: &[SpanRecord], total_us: u64) -> String {
    if spans.is_empty() {
        return String::new();
    }

    let depths = compute_depths(spans);
    let max_depth = depths.iter().copied().max().unwrap_or(0);
    let row_h: u32 = 18;
    let chart_h: u32 = (max_depth + 1) * row_h + 4;
    // Span coordinates are encoded as percentages of the total request
    // duration. The container is `position: relative; width: 100%`, so
    // each rect div lives in a percent-coordinate space. Zooming is
    // re-percenting at runtime via JS — see `__solidev_flame_chart`'s
    // click handler.
    let total = total_us.max(1) as f64;
    let cwd_prefix = std::env::current_dir()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();

    let mut rects = String::new();
    let mut row_html: Vec<String> = Vec::with_capacity(spans.len());
    for (i, s) in spans.iter().enumerate() {
        let dur = s.end_us.saturating_sub(s.start_us).max(1);
        let depth = depths[i];
        let y = depth * row_h;
        let color = flame_color(s.kind);
        let pct = (dur as f64 / total) * 100.0;
        let left_pct = (s.start_us as f64 / total) * 100.0;
        let display_meta = s.meta.as_ref().map(|m| relativize_meta(m, &cwd_prefix));
        let title = match &display_meta {
            Some(m) => format!(
                "{} · {} · {:.1}% · {}",
                s.name,
                fmt_duration_us(dur),
                pct,
                m
            ),
            None => format!("{} · {} · {:.1}%", s.name, fmt_duration_us(dur), pct),
        };
        // HTML divs (not SVG rects) — the browser handles
        // `text-overflow: ellipsis` natively, so labels truncate cleanly
        // when the rect is narrow and reveal more text as the user zooms
        // in. Native `title` attribute provides the hover tooltip with
        // the full name + duration + meta.
        // Same view-pairing attrs as the companion list row, so the
        // overlay-on-hover JS can also light up rendered regions when the
        // user mouses over a rect in the chart.
        let rect_view_attrs = match s.render_id {
            Some(rid) => format!(
                " data-solidev-view-idx=\"{}\" data-solidev-view-name=\"{}\"",
                rid,
                html_escape(&s.name)
            ),
            None => String::new(),
        };
        rects.push_str(&format!(
            "<div class=\"__solidev_rect\" data-idx=\"{idx}\" data-start=\"{ds}\" data-w=\"{dw}\"{view_attrs} title=\"{title}\" style=\"position:absolute;left:{left:.4}%;top:{y}px;width:{w:.4}%;height:{h}px;background:{c};box-sizing:border-box;border-right:1px solid #0b0d0f;font-size:10px;line-height:{h}px;color:#0b0d0f;white-space:nowrap;overflow:hidden;text-overflow:ellipsis;padding:0 4px;cursor:zoom-in;font-family:'JetBrains Mono',ui-monospace,monospace;\">{name}</div>",
            idx = i,
            ds = s.start_us,
            dw = dur,
            view_attrs = rect_view_attrs,
            title = html_escape(&title),
            left = left_pct,
            y = y,
            w = pct,
            h = row_h - 1,
            c = color,
            name = html_escape(&s.name),
        ));

        // Companion list row: depth-indented name + kind badge + duration
        // + %. Hover highlights the matching chart rect (data-idx pairing
        // wired in the inline JS), click zooms.
        let indent_px = depth * 14;
        let meta_html = match &display_meta {
            Some(m) => format!(
                "<span style=\"color:#6c7280;margin-left:0.5rem;\">· {}</span>",
                html_escape(m)
            ),
            None => String::new(),
        };
        // For View/Partial spans, expose the matching `view_log` render id
        // (and template name) so the hover-overlay JS can outline the
        // template's region in the page — same wiring as the view sub-rows
        // in the render-breakdown panel.
        let view_attrs = match s.render_id {
            Some(rid) => format!(
                " data-solidev-view-idx=\"{}\" data-solidev-view-name=\"{}\"",
                rid,
                html_escape(&s.name)
            ),
            None => String::new(),
        };
        row_html.push(format!(
            "<li data-idx=\"{idx}\" data-start=\"{ds}\" data-w=\"{dw}\"{view_attrs} style=\"display:flex;align-items:center;gap:0.5rem;padding:0.125rem 0.25rem;border-radius:0.125rem;cursor:zoom-in;\">\
<span style=\"flex:0 0 auto;width:0.5rem;height:0.5rem;background:{color};border-radius:0.125rem;display:inline-block;\"></span>\
<span style=\"flex:0 0 auto;width:5.5rem;font-size:9px;color:#8b949e;text-transform:uppercase;letter-spacing:0.05em;\">{kind}</span>\
<span style=\"flex:1;color:#e6e6e6;white-space:nowrap;overflow:hidden;text-overflow:ellipsis;padding-left:{indent}px;\">{name}{meta}</span>\
<span style=\"flex:0 0 auto;color:#b8e986;font-variant-numeric:tabular-nums;width:5rem;text-align:right;\">{dur_str}</span>\
<span style=\"flex:0 0 auto;color:#8b949e;font-variant-numeric:tabular-nums;width:3.5rem;text-align:right;\">{pct:.1}%</span>\
</li>",
            idx = i,
            ds = s.start_us,
            dw = dur,
            view_attrs = view_attrs,
            color = color,
            kind = s.kind.as_str(),
            indent = indent_px,
            name = html_escape(&s.name),
            meta = meta_html,
            dur_str = html_escape(&fmt_duration_us(dur)),
            pct = pct,
        ));
    }

    // Spans are captured in close-order (children before parents). Reorder
    // the list to pre-order DFS — parents before their children, siblings
    // in start-time order — so the tree reads top-down.
    let mut order: Vec<usize> = (0..spans.len()).collect();
    order.sort_by(|&a, &b| {
        spans[a]
            .start_us
            .cmp(&spans[b].start_us)
            .then_with(|| spans[b].end_us.cmp(&spans[a].end_us))
    });
    let mut list_rows = String::new();
    for i in order {
        list_rows.push_str(&row_html[i]);
    }

    let trace_json = build_trace_json(spans);
    let trace_b64 = base64::Engine::encode(
        &base64::engine::general_purpose::STANDARD,
        trace_json.as_bytes(),
    );
    let trace_href = format!("data:application/json;base64,{}", trace_b64);

    format!(
        "<div id=\"__solidev_flame\" style=\"display:none;border-top:1px solid #30363d;background:#08090b;padding:0.5rem 0.75rem;\">\
<div style=\"display:flex;align-items:center;gap:0.75rem;margin-bottom:0.5rem;font-size:10px;color:#8b949e;letter-spacing:0.08em;\">\
<span>FLAMEGRAPH · {n_spans} SPAN{plural} · {total_str}</span>\
<span style=\"color:#30363d;\">|</span>\
<span style=\"color:#6c7280;\">click a span to zoom in · double-click the chart to reset</span>\
<a href=\"{href}\" download=\"trace.json\" style=\"margin-left:auto;color:#8be9fd;text-decoration:none;border:1px solid #30363d;padding:0.125rem 0.5rem;border-radius:0.25rem;\">⬇ trace.json</a>\
</div>\
<div id=\"__solidev_flame_chart\" data-total=\"{total_us}\" style=\"position:relative;width:100%;height:{chart_h}px;background:#0b0d0f;overflow:hidden;\">{rects}</div>\
<ol id=\"__solidev_flame_list\" style=\"list-style:none;margin:0.5rem 0 0;padding:0;display:flex;flex-direction:column;gap:0.125rem;font-size:11px;max-height:35vh;overflow-y:auto;\">{list_rows}</ol>\
</div>",
        n_spans = spans.len(),
        plural = if spans.len() == 1 { "" } else { "S" },
        total_str = html_escape(&fmt_duration_us(total_us)),
        total_us = total_us.max(1),
        chart_h = chart_h,
        href = trace_href,
        rects = rects,
        list_rows = list_rows,
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

/// Clickable variant of an aggregate phase row (middleware, view, …).
/// Adds the supplied toggle/chevron ids and `cursor:pointer`; the JS
/// handler in `render_bar` flips the chevron and toggles the matching
/// `*_subrows` container between hidden/shown.
fn aggregate_row_clickable(
    toggle_id: &str,
    chev_id: &str,
    title: &str,
    name: &str,
    us: u64,
    total_us: u64,
    color: &str,
) -> String {
    let pct = if total_us == 0 {
        0
    } else {
        ((us as f64 / total_us as f64) * 100.0).round() as u32
    };
    let bar_width_pct = pct.min(100);
    format!(
        "<li id=\"{toggle_id}\" title=\"{title}\" style=\"display:flex;align-items:center;gap:0.75rem;cursor:pointer;user-select:none;\">\
<span style=\"flex:0 0 5.5rem;color:{color};\"><span id=\"{chev_id}\" style=\"color:#8b949e;font-size:9px;margin-right:0.25rem;\">▶</span>{name}</span>\
<span style=\"flex:0 0 4.5rem;color:#e6e6e6;font-variant-numeric:tabular-nums;text-align:right;\">{dur}</span>\
<span style=\"flex:0 0 2.5rem;color:#8b949e;font-variant-numeric:tabular-nums;text-align:right;\">{pct}%</span>\
<span style=\"flex:1;height:0.5rem;background:#1c1f23;border-radius:0.125rem;overflow:hidden;\"><span style=\"display:block;width:{bar}%;height:100%;background:{color};\"></span></span>\
</li>",
        toggle_id = toggle_id,
        chev_id = chev_id,
        title = html_escape(title),
        color = color,
        name = html_escape(name),
        dur = html_escape(&fmt_duration_us(us)),
        pct = pct,
        bar = bar_width_pct,
    )
}

/// Indented sub-row used under an aggregate row (per middleware, per
/// rendered template). The bar uses the aggregate's colour at reduced
/// opacity so the visual relationship is obvious.
fn sub_row(name: &str, us: u64, total_us: u64, color: &str) -> String {
    let pct = if total_us == 0 {
        0
    } else {
        ((us as f64 / total_us as f64) * 100.0).round() as u32
    };
    let bar_width_pct = pct.min(100);
    format!(
        "<li style=\"display:flex;align-items:center;gap:0.75rem;padding-left:1rem;\">\
<span style=\"flex:0 0 4.5rem;color:#8b949e;font-size:10px;\">└─</span>\
<span style=\"flex:0 0 9rem;color:#e6e6e6;overflow:hidden;text-overflow:ellipsis;\" title=\"{title}\">{name}</span>\
<span style=\"flex:0 0 4.5rem;color:#e6e6e6;font-variant-numeric:tabular-nums;text-align:right;\">{dur}</span>\
<span style=\"flex:0 0 2.5rem;color:#8b949e;font-variant-numeric:tabular-nums;text-align:right;\">{pct}%</span>\
<span style=\"flex:1;height:0.375rem;background:#1c1f23;border-radius:0.125rem;overflow:hidden;\"><span style=\"display:block;width:{bar}%;height:100%;background:{color};opacity:0.7;\"></span></span>\
</li>",
        name = html_escape(name),
        title = html_escape(name),
        dur = html_escape(&fmt_duration_us(us)),
        pct = pct,
        bar = bar_width_pct,
        color = color,
    )
}

/// Sub-row for a rendered view/partial/layout. Differs from `sub_row` by
/// carrying `data-solidev-view-idx`, which the inline hover-overlay
/// script pairs with `<!--solidev:KIND:start id=…-->` markers in the
/// page body. Cursor is set to `pointer` so the row signals interactivity.
/// `depth` (0 = root) controls indentation so the list reads as a tree.
fn view_sub_row(id: u32, depth: u32, name: &str, us: u64, total_us: u64, color: &str) -> String {
    let pct = if total_us == 0 {
        0
    } else {
        ((us as f64 / total_us as f64) * 100.0).round() as u32
    };
    let bar_width_pct = pct.min(100);
    // 1rem base indent + 0.75rem per depth level. Tree branch glyph
    // ('└─') sits in a fixed-width column so siblings stay aligned even
    // when names differ in length.
    let base_indent_rem = 1.0_f32 + 0.75 * depth as f32;
    format!(
        "<li data-solidev-view-idx=\"{id}\" style=\"display:flex;align-items:center;gap:0.75rem;padding-left:{indent}rem;cursor:pointer;\" title=\"hover to outline this template's region in the page\">\
<span style=\"flex:0 0 1.25rem;color:#8b949e;font-size:10px;\">{glyph}</span>\
<span style=\"flex:1 1 auto;color:#e6e6e6;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;\" title=\"{title}\">{name}</span>\
<span style=\"flex:0 0 4.5rem;color:#e6e6e6;font-variant-numeric:tabular-nums;text-align:right;\">{dur}</span>\
<span style=\"flex:0 0 2.5rem;color:#8b949e;font-variant-numeric:tabular-nums;text-align:right;\">{pct}%</span>\
<span style=\"flex:0 0 8rem;height:0.375rem;background:#1c1f23;border-radius:0.125rem;overflow:hidden;\"><span style=\"display:block;width:{bar}%;height:100%;background:{color};opacity:0.7;\"></span></span>\
</li>",
        id = id,
        indent = format!("{:.2}", base_indent_rem),
        glyph = if depth == 0 { "▾" } else { "└─" },
        name = html_escape(name),
        title = html_escape(name),
        dur = html_escape(&fmt_duration_us(us)),
        pct = pct,
        bar = bar_width_pct,
        color = color,
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

fn embed_binds(
    query: &str,
    binds: Option<&std::collections::HashMap<String, serde_json::Value>>,
) -> String {
    let Some(map) = binds else {
        return query.to_string();
    };
    // Substitute longest keys first so `@ab` isn't shadowed by `@a`.
    let mut keys: Vec<&String> = map.keys().collect();
    keys.sort_by_key(|b| std::cmp::Reverse(b.len()));
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
            middlewares: vec![],
            views: vec![],
            spans: vec![],
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
        let out = embed_binds(
            "FOR u IN users FILTER u.name == @name RETURN u",
            Some(&binds),
        );
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
            q(
                "FOR doc IN order_items FILTER doc.order_id == @id RETURN doc",
                0.1,
            ),
            q(
                "FOR doc IN order_items FILTER doc.order_id == @id RETURN doc",
                0.1,
            ),
            q(
                "FOR doc IN order_items FILTER doc.order_id == @id RETURN doc",
                0.1,
            ),
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
    fn renders_per_middleware_subrows_when_multiple() {
        let html = "<html><body></body></html>";
        let mut c = ctx("GET", "/");
        c.phases.push(("middleware".into(), 5_000));
        c.middlewares.push(("auth".into(), 1_500));
        c.middlewares.push(("rate_limit".into(), 2_500));
        c.middlewares.push(("request_id".into(), 1_000));
        let out = inject_dev_bar(html, &c);
        // Aggregate row gets a count badge.
        assert!(out.contains(">middleware (3)<"));
        // Each middleware name renders as a sub-row with the └─ glyph nearby.
        assert!(out.contains(">auth<"));
        assert!(out.contains(">rate_limit<"));
        assert!(out.contains(">request_id<"));
        assert!(out.contains("└─"));
    }

    #[test]
    fn middleware_row_is_clickable_when_subrows_present() {
        let html = "<html><body></body></html>";
        let mut c = ctx("GET", "/");
        c.phases.push(("middleware".into(), 5_000));
        c.middlewares.push(("auth".into(), 5_000));
        let out = inject_dev_bar(html, &c);
        // Aggregate row is rendered with the toggle id + chevron + cursor:pointer.
        assert!(out.contains("id=\"__solidev_mw_toggle\""));
        assert!(out.contains("id=\"__solidev_mw_chev\""));
        assert!(out.contains("cursor:pointer"));
        // Sub-rows wrapper is rendered and starts hidden.
        assert!(out.contains("id=\"__solidev_mw_subrows\""));
    }

    #[test]
    fn middleware_row_not_clickable_without_subrows() {
        let html = "<html><body></body></html>";
        let mut c = ctx("GET", "/");
        c.phases.push(("middleware".into(), 5_000));
        // No middlewares pushed → no toggle row, no subrow wrapper rendered.
        let out = inject_dev_bar(html, &c);
        assert!(!out.contains("id=\"__solidev_mw_toggle\""));
        assert!(!out.contains("id=\"__solidev_mw_subrows\""));
    }

    #[test]
    fn renders_subrow_for_single_middleware() {
        let html = "<html><body></body></html>";
        let mut c = ctx("GET", "/");
        c.phases.push(("middleware".into(), 1_000));
        c.middlewares.push(("auth".into(), 1_000));
        let out = inject_dev_bar(html, &c);
        // Plain "middleware" label, no count badge for a single entry.
        assert!(out.contains(">middleware<"));
        assert!(!out.contains("middleware (1)"));
        // Single middleware still shows its name in a sub-row so the user
        // can see *which* middleware ran.
        assert!(out.contains(">auth<"));
        assert!(out.contains("└─"));
    }

    #[test]
    fn renders_per_template_subrows_when_views_logged() {
        let html = "<html><body></body></html>";
        let mut c = ctx("GET", "/users/42");
        c.phases.push(("view".into(), 5_000));
        // Close-order: child first (card), then parent (show), then layout
        // wrapping the whole thing. Layout has no parent (root), show's
        // parent is the layout, card's parent is show.
        c.views.push((1, Some(0), "users/_card".into(), 800));
        c.views.push((0, Some(2), "users/show".into(), 3_500));
        c.views.push((2, None, "layouts/application".into(), 4_500));
        let out = inject_dev_bar(html, &c);
        // Aggregate row gets a count badge for >1 entries + toggle wiring.
        assert!(out.contains(">view (3)<"));
        assert!(out.contains("id=\"__solidev_view_toggle\""));
        assert!(out.contains("id=\"__solidev_view_chev\""));
        assert!(out.contains("id=\"__solidev_view_subrows\""));
        // Each template name renders as a sub-row.
        assert!(out.contains(">users/show<"));
        assert!(out.contains(">users/_card<"));
        assert!(out.contains(">layouts/application<"));
        // Each view sub-row carries its render id so the hover-overlay
        // JS can pair it with the matching marker comments in the page.
        assert!(out.contains("data-solidev-view-idx=\"0\""));
        assert!(out.contains("data-solidev-view-idx=\"1\""));
        assert!(out.contains("data-solidev-view-idx=\"2\""));
    }

    #[test]
    fn view_row_not_clickable_without_views() {
        let html = "<html><body></body></html>";
        let mut c = ctx("GET", "/");
        c.phases.push(("view".into(), 5_000));
        // No view entries pushed → plain phase_row, no toggle row, no wrapper.
        let out = inject_dev_bar(html, &c);
        assert!(!out.contains("id=\"__solidev_view_toggle\""));
        assert!(!out.contains("id=\"__solidev_view_subrows\""));
        assert!(out.contains(">view<"));
    }

    #[test]
    fn renders_subrow_for_single_view() {
        let html = "<html><body></body></html>";
        let mut c = ctx("GET", "/");
        c.phases.push(("view".into(), 1_000));
        c.views.push((0, None, "home/index".into(), 1_000));
        let out = inject_dev_bar(html, &c);
        // Single view → no count badge, but row is still clickable.
        assert!(out.contains(">view<"));
        assert!(!out.contains("view (1)"));
        assert!(out.contains("id=\"__solidev_view_toggle\""));
        assert!(out.contains(">home/index<"));
        assert!(out.contains("data-solidev-view-idx=\"0\""));
    }

    #[test]
    fn phase_row_handles_zero_total() {
        // When elapsed is 0, the percentage should be 0 not NaN/panic.
        let row = phase_row("view", 0, 0, "#fff");
        assert!(row.contains("0%"));
    }

    fn span(
        id: u32,
        parent: Option<u32>,
        name: &str,
        kind: SpanKind,
        start: u64,
        end: u64,
    ) -> SpanRecord {
        SpanRecord {
            id,
            parent,
            name: name.into(),
            kind,
            start_us: start,
            end_us: end,
            meta: None,
            render_id: None,
        }
    }

    #[test]
    fn flame_panel_absent_when_no_spans() {
        let html = "<html><body></body></html>";
        let c = ctx("GET", "/");
        let out = inject_dev_bar(html, &c);
        assert!(!out.contains("__solidev_flame\""));
        // Button still renders but with `0s` and no extra suffix.
        assert!(out.contains("flame <span"));
    }

    #[test]
    fn flame_panel_renders_one_rect_per_span() {
        let html = "<html><body></body></html>";
        let mut c = ctx("GET", "/");
        c.spans
            .push(span(0, None, "GET /", SpanKind::Action, 0, 12_000));
        c.spans.push(span(
            1,
            Some(0),
            "users/index",
            SpanKind::View,
            1_000,
            9_000,
        ));
        c.spans.push(span(
            2,
            Some(1),
            "users/_card",
            SpanKind::Partial,
            2_500,
            4_000,
        ));
        let out = inject_dev_bar(html, &c);
        assert!(out.contains("id=\"__solidev_flame\""));
        assert!(out.contains("id=\"__solidev_flame_chart\""));
        // One rect-div per span — each carries the `__solidev_rect` class.
        assert_eq!(out.matches("class=\"__solidev_rect\"").count(), 3);
        // Each name shows up: in the rect div's text content and in the
        // `title=` attribute (the hover tooltip).
        assert!(out.contains("users/index"));
        assert!(out.contains("users/_card"));
        // Trace download anchor.
        assert!(out.contains("download=\"trace.json\""));
        assert!(out.contains("data:application/json;base64,"));
    }

    #[test]
    fn flame_panel_assigns_depth_via_parent_chain() {
        let html = "<html><body></body></html>";
        let mut c = ctx("GET", "/");
        c.spans
            .push(span(0, None, "root", SpanKind::Action, 0, 100));
        c.spans
            .push(span(1, Some(0), "child", SpanKind::View, 10, 90));
        c.spans
            .push(span(2, Some(1), "leaf", SpanKind::Partial, 20, 80));
        let depths = compute_depths(&c.spans);
        assert_eq!(depths, vec![0, 1, 2]);
        let out = inject_dev_bar(html, &c);
        // top:0px for root, 18px for child, 36px for leaf — depth × row_h
        assert!(out.contains("top:0px"));
        assert!(out.contains("top:18px"));
        assert!(out.contains("top:36px"));
    }

    #[test]
    fn trace_json_has_one_event_per_span() {
        let spans = vec![
            span(0, None, "GET /", SpanKind::Action, 0, 1000),
            span(1, Some(0), "FOR doc IN posts", SpanKind::Db, 100, 250),
        ];
        let json = build_trace_json(&spans);
        assert!(json.starts_with("{\"traceEvents\":["));
        assert!(json.ends_with("\"displayTimeUnit\":\"ns\"}"));
        assert_eq!(json.matches("\"ph\":\"X\"").count(), 2);
        assert!(json.contains("\"cat\":\"action\""));
        assert!(json.contains("\"cat\":\"db\""));
        assert!(json.contains("\"ts\":100"));
        assert!(json.contains("\"dur\":150"));
    }

    #[test]
    fn relativize_meta_strips_cwd_prefix() {
        assert_eq!(
            relativize_meta("/home/me/proj/app/foo.sl:42", "/home/me/proj"),
            "app/foo.sl:42"
        );
        // Trailing slash on prefix still works.
        assert_eq!(
            relativize_meta("/home/me/proj/app/foo.sl:42", "/home/me/proj/"),
            "app/foo.sl:42"
        );
        // Path outside cwd passes through unchanged.
        assert_eq!(
            relativize_meta("/usr/lib/something.sl:10", "/home/me/proj"),
            "/usr/lib/something.sl:10"
        );
        // Non-path metas (AQL, URLs) pass through unchanged.
        assert_eq!(
            relativize_meta("FOR doc IN users RETURN doc", "/home/me/proj"),
            "FOR doc IN users RETURN doc"
        );
        // Empty cwd prefix is a no-op.
        assert_eq!(relativize_meta("anything", ""), "anything");
    }

    #[test]
    fn json_escape_handles_quotes_and_newlines() {
        assert_eq!(json_escape("a\"b"), "a\\\"b");
        assert_eq!(json_escape("a\nb"), "a\\nb");
        assert_eq!(json_escape("a\\b"), "a\\\\b");
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
