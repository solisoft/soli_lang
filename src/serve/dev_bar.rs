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

use crate::interpreter::builtins::http_log::LoggedHttpRequest;
use crate::interpreter::builtins::kv_log::LoggedKvCall;
use crate::interpreter::builtins::model::query_log::LoggedQuery;
use crate::serve::live_reload::rfind_ascii_case_insensitive;
use crate::serve::span_log::{SpanKind, SpanRecord};

/// Marker injected so we never double-inject (e.g. nested layouts).
const MARKER: &str = "__solidev_bar_injected";

/// Client-side network capture for the "requests" panel, spliced into the dev
/// bar script via the `{net_patch}` placeholder. It wires the header URL button
/// to toggle the requests panel, patches `fetch` + `XMLHttpRequest` ONCE
/// (guarded by `window.__solidevNetHook`, mirroring `__solidevSwapHook`) to
/// append a row per **same-origin** request that carries an `X-Soli-Route`
/// header (i.e. hit a Soli route), and makes each row clickable: a click fetches
/// `/__solidev/request/:id` (the `X-Soli-Request-Id` of that call) and swaps the
/// db/http/kv/flame/breakdown panels + header badges to that request, rebinding
/// the flame + sub-row interactivity on the swapped-in markup.
///
/// `__soBindFlame` / `__soBindSubrows` duplicate the initial inline binding on
/// purpose: the inline code binds the first (main-page) render, and these rebind
/// the fresh elements after a panel swap (a swap replaces the nodes, dropping
/// their listeners). Rows are built with DOM APIs (no innerHTML) and the whole
/// thing uses single-quoted JS, so it stays a raw string with no escaping and no
/// `format!` brace-doubling.
///
/// Structure matters for instant-navigation: nav.js re-executes this inline
/// script after a body swap, but `window` persists — so ONLY the `fetch`/`XHR`
/// monkey-patch is guarded by `window.__solidevNetHook` (patching twice would
/// double-wrap). The DOM bindings (requests-panel toggle, row-click delegation,
/// the bind/select helpers) run on EVERY execution so they re-attach to the new
/// page's bar. The persisted patch calls `window.__soAddReq`, which each run
/// re-points at the current page's list. WebSocket/SSE are not captured.
const NET_PATCH: &str = r#"var reqb=document.getElementById('__solidev_reqbtn');var reqp=document.getElementById('__solidev_requests');if(reqb&&reqp){reqb.addEventListener('click',function(){reqp.style.display=reqp.style.display==='none'?'block':'none';});}function __soSameOrigin(u){try{return new URL(u,location.href).origin===location.origin;}catch(e){return true;}}function __soStatusColor(s){if(s>=200&&s<300)return '#b8e986';if(s>=300&&s<400)return '#f0c674';if(s>=400&&s<500)return '#ffb86c';if(s>=100&&s<200)return '#8be9fd';return '#ff6b6b';}function __soFmtDur(ms){if(ms<1)return Math.round(ms*1000)+'µs';if(ms<1000)return ms.toFixed(1)+'ms';return (ms/1000).toFixed(2)+'s';}function __soCell(t,css){var s=document.createElement('span');s.style.cssText=css;s.textContent=t;return s;}function __soEsc(s){return String(s).replace(/[&<>"]/g,function(c){return c==='&'?'&amp;':c==='<'?'&lt;':c==='>'?'&gt;':'&quot;';});}window.__soUpdateHeader=function(method,path,status,route,appUs){var rc=document.getElementById('__solidev_render_count');if(rc&&appUs&&appUs>0)rc.textContent=__soFmtDur(appUs/1000);var btn=document.getElementById('__solidev_reqbtn');if(btn){var cnt=document.getElementById('__solidev_req_count');var cntTxt=cnt?cnt.textContent:'';var arrow=route?(' <span style="color:#8b949e;">→</span> <span style="color:#c586e9;">'+__soEsc(route)+'</span>'):'';btn.innerHTML='<span style="color:#8be9fd;">'+__soEsc(method)+'</span> <span style="color:#e6e6e6;">'+__soEsc(path)+'</span> <span style="color:#8b949e;">[</span><span style="color:'+__soStatusColor(status)+';">'+__soEsc(String(status))+'</span><span style="color:#8b949e;">]</span>'+arrow+' <span id="__solidev_req_count" style="color:#8b949e;">'+__soEsc(cntTxt)+'</span>';}};window.__soAddReq=function(method,url,status,appUs,rtMs,route,rid){var list=document.getElementById('__solidev_req_list');if(!list)return;var li=document.createElement('li');li.className='__solidev_req_li';if(rid)li.setAttribute('data-req-id',rid);li.title='click to inspect this request (db / http / kv / flame)';li.style.cssText='display:flex;flex-wrap:wrap;align-items:center;gap:0.25rem 0.75rem;cursor:pointer;padding:0.15rem 0.25rem;border-radius:0.25rem;';li.setAttribute('data-method',String(method));li.setAttribute('data-route',route||'');li.setAttribute('data-status',String(status));li.setAttribute('data-app-us',String(appUs||0));try{li.setAttribute('data-path',new URL(url,location.href).pathname);}catch(e){li.setAttribute('data-path',url);}li.appendChild(__soCell(url,'flex:1 1 100%;order:-1;color:#e6e6e6;word-break:break-all;'));li.appendChild(__soCell(method,'flex:0 0 auto;color:#8be9fd;'));li.appendChild(__soCell(route,'flex:0 0 auto;color:#c586e9;'));li.appendChild(__soCell(String(status),'flex:0 0 auto;color:'+__soStatusColor(status)+';font-variant-numeric:tabular-nums;'));var hasApp=appUs&&appUs>0;var dur=__soCell(hasApp?__soFmtDur(appUs/1000):__soFmtDur(rtMs),'flex:0 0 auto;color:#b8e986;font-variant-numeric:tabular-nums;');dur.title=(hasApp?'app render '+__soFmtDur(appUs/1000):'app render n/a')+' · round-trip '+__soFmtDur(rtMs);li.appendChild(dur);if(rid){var rpb=__soCell('↻','flex:0 0 auto;color:#8b949e;cursor:pointer;user-select:none;');rpb.className='__solidev_replay';rpb.setAttribute('data-req-id',rid);rpb.title='replay this request server-side';li.appendChild(rpb);}list.appendChild(li);var n=list.querySelectorAll('.__solidev_req_li').length;var hc=document.getElementById('__solidev_req_count');if(hc)hc.textContent='('+n+')';var pc=document.getElementById('__solidev_req_hdr_count');if(pc)pc.textContent=n;};var __now=function(){return (window.performance&&performance.now)?performance.now():Date.now();};function __soBindFlame(){var fchart=document.getElementById('__solidev_flame_chart');var flist=document.getElementById('__solidev_flame_list');if(!fchart)return;var totalUs=parseFloat(fchart.getAttribute('data-total'))||1;var rects=fchart.querySelectorAll('.__solidev_rect');function applyZoom(viewStart,viewW){rects.forEach(function(r){var s=parseFloat(r.getAttribute('data-start'));var w=parseFloat(r.getAttribute('data-w'));var rs=s-viewStart;var re=rs+w;if(re<=0||rs>=viewW){r.style.display='none';return;}r.style.display='';var cs=Math.max(0,rs);var ce=Math.min(viewW,re);r.style.left=(cs/viewW*100)+'%';r.style.width=Math.max(0.001,(ce-cs)/viewW*100)+'%';});}function hr(rect,on){if(!rect)return;rect.style.outline=on?'2px solid #ffffff':'';rect.style.outlineOffset=on?'-2px':'';}function hl(li,on){if(!li)return;li.style.background=on?'#1c1f23':'';}rects.forEach(function(r){r.addEventListener('click',function(ev){ev.stopPropagation();applyZoom(parseFloat(r.getAttribute('data-start')),parseFloat(r.getAttribute('data-w')));});r.addEventListener('mouseenter',function(){var idx=r.getAttribute('data-idx');var li=flist?flist.querySelector('li[data-idx="'+idx+'"]'):null;hl(li,true);hr(r,true);});r.addEventListener('mouseleave',function(){var idx=r.getAttribute('data-idx');var li=flist?flist.querySelector('li[data-idx="'+idx+'"]'):null;hl(li,false);hr(r,false);});});fchart.addEventListener('dblclick',function(){applyZoom(0,totalUs);});if(flist){flist.querySelectorAll('li[data-idx]').forEach(function(li){li.addEventListener('mouseenter',function(){var idx=li.getAttribute('data-idx');hl(li,true);hr(fchart.querySelector('.__solidev_rect[data-idx="'+idx+'"]'),true);});li.addEventListener('mouseleave',function(){var idx=li.getAttribute('data-idx');hl(li,false);hr(fchart.querySelector('.__solidev_rect[data-idx="'+idx+'"]'),false);});li.addEventListener('click',function(){applyZoom(parseFloat(li.getAttribute('data-start')),parseFloat(li.getAttribute('data-w')));});});}}function __soBindSubrows(){var mwt=document.getElementById('__solidev_mw_toggle');var mws=document.getElementById('__solidev_mw_subrows');var mwc=document.getElementById('__solidev_mw_chev');if(mwt&&mws){mwt.addEventListener('click',function(){var h=mws.style.display==='none';mws.style.display=h?'':'none';if(mwc)mwc.textContent=h?'▼':'▶';});}var vwt=document.getElementById('__solidev_view_toggle');var vws=document.getElementById('__solidev_view_subrows');var vwc=document.getElementById('__solidev_view_chev');if(vwt&&vws){vwt.addEventListener('click',function(){var h=vws.style.display==='none';vws.style.display=h?'':'none';if(vwc)vwc.textContent=h?'▼':'▶';});}}function __soSelectReq(id,row){var list=document.getElementById('__solidev_req_list');if(list)Array.prototype.forEach.call(list.querySelectorAll('.__solidev_req_li'),function(li){li.style.background=(li===row)?'#1c1f23':'';});if(!id)return;fetch('/__solidev/request/'+encodeURIComponent(id)).then(function(r){return r.ok?r.text():null;}).then(function(html){if(!html)return;var tmp=document.createElement('div');tmp.innerHTML=html;var np=tmp.querySelector('#__solidev_panels');var lp=document.getElementById('__solidev_panels');if(np&&lp){var openIds=[];['__solidev_phases','__solidev_queries','__solidev_http','__solidev_kv','__solidev_flame'].forEach(function(pid){var el=document.getElementById(pid);if(el&&el.style.display!=='none')openIds.push(pid);});lp.innerHTML=np.innerHTML;openIds.forEach(function(pid){var el=document.getElementById(pid);if(el)el.style.display='block';});__soBindFlame();__soBindSubrows();}['__solidev_render_count','__solidev_db_count','__solidev_http_count','__solidev_kv_count','__solidev_flame_count'].forEach(function(bid){var s=tmp.querySelector('#'+bid);var d=document.getElementById(bid);if(s&&d)d.textContent=s.textContent;});}).catch(function(e){});}function __soReplay(id,btn){if(!id)return;var prev=btn?btn.textContent:'';if(btn){btn.textContent='…';btn.style.color='#f0c674';}fetch('/__solidev/replay/'+encodeURIComponent(id),{method:'POST'}).then(function(r){var nid='';try{nid=r.headers.get('X-Soli-Request-Id')||'';}catch(e){}if(btn){btn.textContent=prev||'↻';btn.style.color='#8b949e';}if(nid){var row=document.querySelector('#__solidev_req_list .__solidev_req_li[data-req-id=\"'+nid+'\"]');__soSelectReq(nid,row||null);}}).catch(function(e){if(btn){btn.textContent=prev||'↻';btn.style.color='#ff6b6b';}});}var reqList=document.getElementById('__solidev_req_list');if(reqList){reqList.addEventListener('click',function(ev){var rpx=ev.target&&ev.target.closest?ev.target.closest('.__solidev_replay'):null;if(rpx&&reqList.contains(rpx)){ev.stopPropagation();__soReplay(rpx.getAttribute('data-req-id'),rpx);return;}var li=ev.target&&ev.target.closest?ev.target.closest('.__solidev_req_li'):null;if(li&&reqList.contains(li))__soSelectReq(li.getAttribute('data-req-id'),li);});}if(!window.__solidevNetHook){window.__solidevNetHook=1;var __of=window.fetch;if(__of){window.fetch=function(input,init){var url=(typeof input==='string')?input:(input&&input.url)||'';var method=(init&&init.method)||(input&&input.method)||'GET';var t0=__now();return __of.apply(this,arguments).then(function(resp){try{if(__soSameOrigin(url)&&url.indexOf('/__solidev/')<0){var r='';var id='';var au=0;try{r=resp.headers.get('X-Soli-Route')||'';id=resp.headers.get('X-Soli-Request-Id')||'';au=parseFloat(resp.headers.get('X-Soli-Render-Us')||'0');}catch(e){}if(r&&window.__soAddReq)window.__soAddReq(String(method).toUpperCase(),url,resp.status,au,__now()-t0,r,id);}}catch(e){}return resp;});};}var __oo=XMLHttpRequest.prototype.open;var __os=XMLHttpRequest.prototype.send;XMLHttpRequest.prototype.open=function(m,u){this.__soM=m;this.__soU=u;return __oo.apply(this,arguments);};XMLHttpRequest.prototype.send=function(){var xhr=this;xhr.__soT0=__now();xhr.addEventListener('loadend',function(){try{if(__soSameOrigin(xhr.__soU)&&String(xhr.__soU).indexOf('/__solidev/')<0){var r='';var id='';var au=0;try{r=xhr.getResponseHeader('X-Soli-Route')||'';id=xhr.getResponseHeader('X-Soli-Request-Id')||'';au=parseFloat(xhr.getResponseHeader('X-Soli-Render-Us')||'0');}catch(e){}if(r&&window.__soAddReq)window.__soAddReq(String(xhr.__soM||'GET').toUpperCase(),xhr.__soU,xhr.status,au,__now()-xhr.__soT0,r,id);}}catch(e){}});return __os.apply(this,arguments);};}if(!window.__solidevHtmxHook){window.__solidevHtmxHook=1;var __soHdrFromPush=function(e){try{if(!window.__soUpdateHeader)return;var np=(e&&e.detail&&e.detail.path)||location.pathname;np=String(np).split('?')[0].split('#')[0];var rows=document.querySelectorAll('#__solidev_req_list .__solidev_req_li');for(var i=rows.length-1;i>=0;i--){if(rows[i].getAttribute('data-path')===np){var r=rows[i];window.__soUpdateHeader(r.getAttribute('data-method')||'GET',np,parseInt(r.getAttribute('data-status'))||0,r.getAttribute('data-route')||'',parseFloat(r.getAttribute('data-app-us'))||0);break;}}}catch(err){}};document.addEventListener('htmx:pushedIntoHistory',__soHdrFromPush);document.addEventListener('htmx:replacedIntoHistory',__soHdrFromPush);document.addEventListener('htmx:replacedInHistory',__soHdrFromPush);}"#;

/// Return true when the incoming request is an HTMx partial swap. HTMx sets
/// `HX-Request: true` on every fragment fetch; the live page already carries
/// a dev bar, so the fragment must not include one too. Request header names
/// arrive lowercased from hyper (see `extract_headers`).
/// `hx_request` is the value of the `hx-request` header, if present.
pub fn is_htmx_request(hx_request: Option<&str>) -> bool {
    hx_request
        .map(|v| v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

/// Data the response thread captures and hands to the injector.
/// A fully-owned, cloneable snapshot of one request's dev data. It is both the
/// input to [`render_bar`] and the value stored per-request in
/// [`crate::serve::dev_store`], so a later `/__solidev/request/:id` fetch can
/// re-render this exact request's panels for the dev bar's "requests" drill-down.
#[derive(Clone)]
pub struct DevBarContext {
    pub method: String,
    pub path: String,
    pub status: u16,
    /// Total request wall-clock, frozen at finalize (not a live `Instant`, so
    /// the snapshot re-renders with the request's real duration later).
    pub elapsed_us: u64,
    /// Stable per-request id (matches the `X-Soli-Request-Id` header). Row 0 of
    /// the requests panel carries it so a click can re-fetch this request.
    pub request_id: String,
    /// Matched route for this request as `controller#action` (or a bare
    /// function name). `None` for unmatched (404) responses — the requests
    /// panel's self-row then shows `—` for the route.
    pub route: Option<String>,
    pub queries: Vec<LoggedQuery>,
    pub http_requests: Vec<LoggedHttpRequest>,
    /// One entry per SoliKV / Cache command (`KV.*` / `Cache.*`) in the
    /// order they fired. Feeds the dev bar's "kv" panel.
    pub kv_calls: Vec<LoggedKvCall>,
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
    /// Developer warnings raised during render (e.g. a component declared a
    /// prop that wasn't provided). Empty in non-dev mode. Shown in the
    /// Warnings panel + a header badge.
    pub warnings: Vec<String>,
}

/// Render the dev bar for a single stored request, used by the
/// `/__solidev/request/:id` endpoint. The client parses the returned markup and
/// swaps `#__solidev_panels` (and the header badge counts) into the live bar so
/// the panels retarget to the clicked request. Returns the same markup
/// `inject_dev_bar` would splice, minus the wrapping — the client only reads
/// `#__solidev_panels` and the `*_count` spans out of it.
pub fn render_for_inspect(ctx: &DevBarContext) -> String {
    render_bar(ctx)
}

/// Inject the dev bar into an HTML body. Idempotent: returns input unchanged
/// if the marker is already present.
pub fn inject_dev_bar(html: &str, ctx: &DevBarContext) -> String {
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

fn render_bar(ctx: &DevBarContext) -> String {
    let elapsed_us = ctx.elapsed_us;
    let render_str = fmt_duration_us(elapsed_us);
    let rss_str = read_rss_str();

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
    // Aggregate view time = sum of the ROOT view spans (those whose parent
    // isn't in this snapshot). Each root span already includes its nested
    // partials/layout, so summing roots — not every entry — avoids
    // double-counting. We derive this from the per-template spans
    // (`ctx.views`) the breakdown already renders, NOT the "view" phase
    // marker: that marker isn't emitted on every render path, which left the
    // aggregate row at 0ms while the sub-rows showed real time. Fall back to
    // the phase total only when there are no view spans.
    let view_us: u64 = if ctx.views.is_empty() {
        ctx.phases
            .iter()
            .filter(|(k, _)| k == "view")
            .map(|(_, v)| *v)
            .sum()
    } else {
        let id_set: std::collections::HashSet<u32> =
            ctx.views.iter().map(|(id, _, _, _)| *id).collect();
        ctx.views
            .iter()
            .filter(|(_, parent, _, _)| match parent {
                Some(p) => !id_set.contains(p),
                None => true,
            })
            .map(|(_, _, _, us)| *us)
            .sum()
    };
    let h_us_total: u64 = ctx
        .http_requests
        .iter()
        .map(|r| (r.duration_ms * 1000.0).max(0.0) as u64)
        .sum();
    let kv_us_total: u64 = ctx
        .kv_calls
        .iter()
        .map(|c| (c.duration_ms * 1000.0).max(0.0) as u64)
        .sum();
    let measured_us = mw_us
        .saturating_add(view_us)
        .saturating_add(q_total_us)
        .saturating_add(h_us_total)
        .saturating_add(kv_us_total);
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
        "<div id=\"__solidev_phases\" class=\"__solidev_panel\" style=\"display:none;border-top:1px solid #30363d;background:#08090b;padding:0.5rem 0.75rem;max-height:33vh;overflow-y:auto;\">\
<div style=\"margin-bottom:0.5rem;font-size:10px;color:#8b949e;letter-spacing:0.08em;\">RENDER · {total}</div>\
<ol style=\"list-style:none;margin:0;padding:0;display:flex;flex-direction:column;gap:0.25rem;font-size:11px;\">\
{mw_row}\
{mw_sub_rows}\
{ctrl_row}\
{view_row}\
{view_sub_rows}\
{db_row}\
{http_row}\
{kv_row}\
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
        kv_row = phase_row("kv", kv_us_total, elapsed_us, "#ffb86c"),
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
<span class=\"__solidev_q_dur\" style=\"flex:0 0 auto;color:#b8e986;width:5rem;text-align:right;font-variant-numeric:tabular-nums;\">{}</span>\
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
            "<div id=\"__solidev_queries\" class=\"__solidev_panel\" style=\"display:none;border-top:1px solid #30363d;background:#08090b;max-height:40vh;overflow-y:auto;padding:0.5rem 0.75rem;\">\
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
    // Surface N+1 on the minimized "DEV" pill so the issue is visible even
    // when the bar is collapsed. Stays inside the same button (the whole pill
    // re-opens the bar on click).
    let n1_minimized_badge = if has_n1 {
        "<span style=\"color:#ff6b6b;font-weight:600;margin-left:0.375rem;\" title=\"N+1 detected · click to open dev bar\">N+1</span>"
    } else {
        ""
    };

    // Developer warnings raised during render (e.g. component missing prop). An
    // always-visible red block at the top of the panels + a badge on the pill.
    let warnings_panel = if ctx.warnings.is_empty() {
        String::new()
    } else {
        let mut items = String::new();
        for w in &ctx.warnings {
            items.push_str(&format!(
                "<li style=\"padding:0.2rem 0;color:#ffb86c;\">{}</li>",
                html_escape(w)
            ));
        }
        format!(
            "<div id=\"__solidev_warnings\" style=\"border-top:1px solid #30363d;background:#2a0d0f;border-left:3px solid #ff6b6b;padding:0.5rem 0.75rem;\">\
<div style=\"margin-bottom:0.375rem;font-size:10px;color:#ff6b6b;letter-spacing:0.08em;\">\u{26a0} {} WARNING{}</div>\
<ol style=\"list-style:none;margin:0;padding:0;display:flex;flex-direction:column;gap:0.25rem;font-size:11px;\">{}</ol>\
</div>",
            ctx.warnings.len(),
            if ctx.warnings.len() == 1 { "" } else { "S" },
            items,
        )
    };
    let warn_minimized_badge = if ctx.warnings.is_empty() {
        String::new()
    } else {
        format!(
            "<span style=\"color:#ffb86c;font-weight:600;margin-left:0.375rem;\" title=\"{} warning(s) \u{b7} click to open dev bar\">\u{26a0}{}</span>",
            ctx.warnings.len(),
            ctx.warnings.len(),
        )
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
                "<li class=\"__solidev_http_li\" style=\"display:flex;flex-wrap:wrap;align-items:center;gap:0.25rem 0.75rem;\">\
<span class=\"__solidev_http_url\" style=\"flex:1 1 100%;order:-1;color:#e6e6e6;word-break:break-all;user-select:all;\">{url}</span>\
<span class=\"__solidev_http_method\" style=\"flex:0 0 auto;color:#8be9fd;\">{method}</span>\
<span class=\"__solidev_http_status\" style=\"flex:0 0 auto;color:{sc};font-variant-numeric:tabular-nums;\">{slabel}</span>\
<span class=\"__solidev_http_dur\" style=\"flex:0 0 auto;color:#b8e986;font-variant-numeric:tabular-nums;\">{dur}</span>\
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
            "<div id=\"__solidev_http\" class=\"__solidev_panel\" style=\"display:none;border-top:1px solid #30363d;background:#08090b;max-height:40vh;overflow-y:auto;padding:0.5rem 0.75rem;\">\
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

    // KV panel: SoliKV / Cache commands fired during this request. Mirrors
    // the HTTP panel — verb + key + duration, one row per command. Values
    // are never captured (see `kv_log`), so only the verb and key show.
    let kv_count = ctx.kv_calls.len();
    let kv_total_str = fmt_duration_us(kv_us_total);

    let kv_panel = if kv_count == 0 {
        String::new()
    } else {
        let mut rows = String::new();
        for c in &ctx.kv_calls {
            let dur_us = (c.duration_ms * 1000.0).max(0.0) as u64;
            let dur = fmt_duration_us(dur_us);
            let (status_label, status_color) = match &c.error {
                Some(err) => (format!("ERR: {}", err), "#ff6b6b"),
                None => ("OK".to_string(), "#b8e986"),
            };
            rows.push_str(&format!(
                "<li class=\"__solidev_kv_li\" style=\"display:flex;flex-wrap:wrap;align-items:center;gap:0.25rem 0.75rem;\">\
<span class=\"__solidev_kv_key\" style=\"flex:1 1 100%;order:-1;color:#e6e6e6;word-break:break-all;user-select:all;\">{key}</span>\
<span class=\"__solidev_kv_cmd\" style=\"flex:0 0 auto;color:#ffb86c;\">{cmd}</span>\
<span class=\"__solidev_kv_status\" style=\"flex:0 0 auto;color:{sc};font-variant-numeric:tabular-nums;\">{slabel}</span>\
<span class=\"__solidev_kv_dur\" style=\"flex:0 0 auto;color:#b8e986;font-variant-numeric:tabular-nums;\">{dur}</span>\
</li>",
                dur = html_escape(&dur),
                sc = status_color,
                slabel = html_escape(&status_label),
                cmd = html_escape(&c.command),
                key = html_escape(&c.key),
            ));
        }
        let plural = if kv_count == 1 { "" } else { "S" };
        format!(
            "<div id=\"__solidev_kv\" class=\"__solidev_panel\" style=\"display:none;border-top:1px solid #30363d;background:#08090b;max-height:40vh;overflow-y:auto;padding:0.5rem 0.75rem;\">\
<div style=\"margin-bottom:0.5rem;font-size:10px;color:#8b949e;letter-spacing:0.08em;\">KV · {} COMMAND{} · {}</div>\
<ol style=\"list-style:none;margin:0;padding:0;display:flex;flex-direction:column;gap:0.375rem;font-size:11px;\">{}</ol>\
</div>",
            kv_count,
            plural,
            html_escape(&kv_total_str),
            rows,
        )
    };

    let kv_btn_extra = if kv_count > 0 {
        format!(
            "<span style=\"color:#8b949e;\"> · </span><span style=\"color:#b8e986;\">{}</span>",
            html_escape(&kv_total_str)
        )
    } else {
        String::new()
    };

    // Flamegraph panel: hierarchical view of every captured span. Empty
    // when the request didn't open any spans (e.g. dev mode off, or a
    // 404 with no controller dispatch).
    let flame_count = ctx.spans.len();
    let flame_panel = render_flame_panel(&ctx.spans, elapsed_us);

    // Requests panel: the page's own route (server-rendered self-row) plus the
    // XHR/fetch/HTMx calls the page fires afterwards, appended client-side by
    // the fetch/XHR patch in the script below. Toggled by clicking the URL in
    // the header. `route_arrow` shows the current route next to the path.
    let route_label = ctx.route.as_deref().unwrap_or("—");
    let route_arrow = if ctx.route.is_some() {
        format!(
            "<span style=\"color:#8b949e;\"> → </span><span style=\"color:#c586e9;\">{}</span>",
            html_escape(route_label)
        )
    } else {
        String::new()
    };
    let requests_panel = format!(
        "<div id=\"__solidev_requests\" class=\"__solidev_panel\" style=\"display:none;border-top:1px solid #30363d;background:#08090b;max-height:40vh;overflow-y:auto;padding:0.5rem 0.75rem;\">\
<div style=\"margin-bottom:0.5rem;font-size:10px;color:#8b949e;letter-spacing:0.08em;\">REQUESTS · <span id=\"__solidev_req_hdr_count\">1</span> · main page + same-origin XHR/fetch · time = app render (hover for round-trip)</div>\
<ol id=\"__solidev_req_list\" style=\"list-style:none;margin:0;padding:0;display:flex;flex-direction:column;gap:0.375rem;font-size:11px;\">\
<li class=\"__solidev_req_li\" data-req-id=\"{rid}\" title=\"click to inspect this request's panels (db / http / kv / flame)\" style=\"display:flex;flex-wrap:wrap;align-items:center;gap:0.25rem 0.75rem;cursor:pointer;padding:0.15rem 0.25rem;border-radius:0.25rem;background:#1c1f23;\">\
<span class=\"__solidev_req_url\" style=\"flex:1 1 100%;order:-1;color:#e6e6e6;word-break:break-all;\">{path}</span>\
<span class=\"__solidev_req_method\" style=\"flex:0 0 auto;color:#8be9fd;\">{method}</span>\
<span class=\"__solidev_req_route\" style=\"flex:0 0 auto;color:#c586e9;\">{route}</span>\
<span class=\"__solidev_req_status\" style=\"flex:0 0 auto;color:{sc};font-variant-numeric:tabular-nums;\">{status}</span>\
<span class=\"__solidev_req_dur\" style=\"flex:0 0 auto;color:#b8e986;font-variant-numeric:tabular-nums;\">{dur}</span>\
<span class=\"__solidev_replay\" data-req-id=\"{rid}\" title=\"replay this request server-side\" style=\"flex:0 0 auto;color:#8b949e;cursor:pointer;user-select:none;\">\u{21bb}</span>\
</li>\
</ol>\
</div>",
        rid = html_escape(&ctx.request_id),
        path = html_escape(&ctx.path),
        method = html_escape(&ctx.method),
        route = html_escape(route_label),
        sc = status_color,
        status = status,
        dur = html_escape(&render_str),
    );

    format!(
        "<!-- {marker} -->\
<style>.__solidev_icon{{display:none}}.__solidev_mob{{display:inline-flex!important;align-items:center;gap:0.375rem;line-height:1.4}}@media(max-width:600px){{.__solidev_icon{{display:inline-flex!important;vertical-align:middle;margin:0!important}}.__solidev_label{{display:none!important}}.__solidev_mob{{padding:0.125rem 0.25rem!important;gap:0.25rem!important;font-size:11px!important}}#__solidev_bar{{max-height:80vh}}.__solidev_panel{{padding:0.5rem 0.5rem!important}}.__solidev_pr{{gap:0.5rem!important}}.__solidev_pr_name{{flex:0 0 4.5rem!important;font-size:10px}}.__solidev_pr_dur{{flex:0 0 3.5rem!important;font-size:10px}}.__solidev_col_pct{{display:none!important}}.__solidev_sub_glyph{{flex:0 0 1rem!important;font-size:9px}}.__solidev_sub_name{{flex:1 1 auto!important;min-width:0;overflow:hidden;text-overflow:ellipsis;white-space:nowrap}}.__solidev_sub_dur{{flex:0 0 3.5rem!important;font-size:10px}}.__solidev_sub_bar{{display:none!important}}.__solidev_view_sub_bar{{display:none!important}}.__solidev_view_sub_dur{{flex:0 0 3.5rem!important;font-size:10px}}.__solidev_http_li{{gap:0.25rem 0.5rem!important}}.__solidev_http_dur,.__solidev_http_status,.__solidev_http_method,.__solidev_http_url{{font-size:10px!important}}.__solidev_kv_li{{gap:0.25rem 0.5rem!important}}.__solidev_kv_dur,.__solidev_kv_status,.__solidev_kv_cmd,.__solidev_kv_key{{font-size:10px!important}}.__solidev_q_dur{{width:4rem!important;font-size:10px}}.__solidev_flame_li{{gap:0.375rem!important}}.__solidev_flame_kind{{width:3.5rem!important;font-size:8px!important}}.__solidev_flame_dur{{width:4rem!important;font-size:10px}}.__solidev_flame_head{{flex-wrap:wrap!important;gap:0.375rem!important}}.__solidev_flame_help{{display:none!important}}}}</style>\
<aside id=\"__solidev_bar\" data-current-req=\"{request_id}\" style=\"position:fixed;bottom:0;left:0;right:0;z-index:2147483646;font-family:'JetBrains Mono',ui-monospace,SFMono-Regular,Menlo,Consolas,monospace;font-size:12px;background:#0b0d0f;color:#c9d1d9;border-top:1px solid #30363d;max-height:100vh;overflow-y:auto;\">\
<div style=\"display:flex;flex-wrap:wrap;align-items:center;column-gap:0.75rem;row-gap:0.25rem;padding:0.375rem 2rem 0.375rem 0.75rem;position:sticky;top:0;background:#0b0d0f;z-index:1;border-bottom:1px solid #30363d;\">\
<button type=\"button\" id=\"__solidev_close\" aria-label=\"Hide dev bar (Alt+D)\" title=\"hide (Alt+D)\" style=\"position:absolute;top:0.25rem;right:0.375rem;z-index:2;padding:0 0.5rem;border-radius:0.25rem;color:#c9d1d9;font:inherit;cursor:pointer;border:none;background:transparent;\">×</button>\
<div style=\"display:flex;align-items:center;gap:0.5rem;flex:1 1 auto;min-width:180px;\">\
<span style=\"padding:0 0.375rem;border-radius:0.25rem;background:#3a2a00;color:#f0c674;flex:0 0 auto;\" title=\"APP_ENV\">DEV</span>\
<button type=\"button\" id=\"__solidev_reqbtn\" title=\"click to list all routes/requests for this page (main + XHR)\" style=\"min-width:0;flex:1 1 auto;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;text-align:left;font:inherit;border:none;background:transparent;color:inherit;cursor:pointer;padding:0;\"><span style=\"color:#8be9fd;\">{method}</span> <span style=\"color:#e6e6e6;\">{path}</span> <span style=\"color:#8b949e;\">[</span><span style=\"color:{status_color};\">{status}</span><span style=\"color:#8b949e;\">]</span>{route_arrow} <span id=\"__solidev_req_count\" style=\"color:#8b949e;\">(1)</span></button>\
</div>\
<div style=\"display:flex;align-items:center;gap:0.5rem;flex:0 1 auto;flex-wrap:wrap;\">\
<button type=\"button\" id=\"__solidev_rb\" class=\"__solidev_mob\" title=\"click to expand render breakdown (middleware / controller / view / db / http)\" style=\"padding:0 0.25rem;border-radius:0.25rem;color:#c9d1d9;font:inherit;cursor:pointer;border:none;background:transparent;\"><svg class=\"__solidev_icon\" width=\"14\" height=\"14\" viewBox=\"0 0 24 24\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"2\" stroke-linecap=\"round\" stroke-linejoin=\"round\"><circle cx=\"12\" cy=\"12\" r=\"10\"/><polyline points=\"12 6 12 12 16 14\"/></svg><span class=\"__solidev_label\">render</span> <span id=\"__solidev_render_count\" style=\"color:#b8e986;\">{render}</span></button>\
<span style=\"color:#30363d;\">|</span>\
<span class=\"__solidev_mob\" title=\"resident memory of this worker\" style=\"padding:0 0.25rem;border-radius:0.25rem;color:#c9d1d9;background:transparent;\"><svg class=\"__solidev_icon\" width=\"14\" height=\"14\" viewBox=\"0 0 24 24\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"2\" stroke-linecap=\"round\" stroke-linejoin=\"round\"><rect x=\"4\" y=\"4\" width=\"16\" height=\"16\" rx=\"2\"/><rect x=\"9\" y=\"9\" width=\"6\" height=\"6\"/><path d=\"M9 1v3M15 1v3M9 20v3M15 20v3M20 9h3M20 14h3M1 9h3M1 14h3\"/></svg><span class=\"__solidev_label\">rss</span> <span style=\"color:#b8e986;\">{rss}</span></span>\
<span style=\"color:#30363d;\">|</span>\
<button type=\"button\" id=\"__solidev_db\" class=\"__solidev_mob\" title=\"click to expand SolidB queries for this request\" style=\"padding:0 0.25rem;border-radius:0.25rem;color:{db_label_color};font:inherit;cursor:pointer;border:none;background:transparent;\"><svg class=\"__solidev_icon\" width=\"14\" height=\"14\" viewBox=\"0 0 24 24\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"2\" stroke-linecap=\"round\" stroke-linejoin=\"round\"><ellipse cx=\"12\" cy=\"5\" rx=\"9\" ry=\"3\"/><path d=\"M21 12c0 1.66-4 3-9 3s-9-1.34-9-3\"/><path d=\"M3 5v14c0 1.66 4 3 9 3s9-1.34 9-3V5\"/></svg><span class=\"__solidev_label\">db</span> <span id=\"__solidev_db_count\" style=\"color:#b8e986;\">{q_count}q</span>{q_btn_extra}</button>\
<span style=\"color:#30363d;\">|</span>\
<button type=\"button\" id=\"__solidev_hb\" class=\"__solidev_mob\" title=\"click to expand outgoing HTTP requests for this request\" style=\"padding:0 0.25rem;border-radius:0.25rem;color:#c9d1d9;font:inherit;cursor:pointer;border:none;background:transparent;\"><svg class=\"__solidev_icon\" width=\"14\" height=\"14\" viewBox=\"0 0 24 24\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"2\" stroke-linecap=\"round\" stroke-linejoin=\"round\"><circle cx=\"12\" cy=\"12\" r=\"10\"/><line x1=\"2\" y1=\"12\" x2=\"22\" y2=\"12\"/><path d=\"M12 2a15.3 15.3 0 0 1 4 10 15.3 15.3 0 0 1-4 10 15.3 15.3 0 0 1-4-10 15.3 15.3 0 0 1 4-10z\"/></svg><span class=\"__solidev_label\">http</span> <span id=\"__solidev_http_count\" style=\"color:#b8e986;\">{h_count}r</span>{h_btn_extra}</button>\
<span style=\"color:#30363d;\">|</span>\
<button type=\"button\" id=\"__solidev_kb\" class=\"__solidev_mob\" title=\"click to expand SoliKV / Cache commands for this request\" style=\"padding:0 0.25rem;border-radius:0.25rem;color:#c9d1d9;font:inherit;cursor:pointer;border:none;background:transparent;\"><svg class=\"__solidev_icon\" width=\"14\" height=\"14\" viewBox=\"0 0 24 24\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"2\" stroke-linecap=\"round\" stroke-linejoin=\"round\"><rect x=\"2\" y=\"2\" width=\"20\" height=\"8\" rx=\"2\" ry=\"2\"/><rect x=\"2\" y=\"14\" width=\"20\" height=\"8\" rx=\"2\" ry=\"2\"/><line x1=\"6\" y1=\"6\" x2=\"6.01\" y2=\"6\"/><line x1=\"6\" y1=\"18\" x2=\"6.01\" y2=\"18\"/></svg><span class=\"__solidev_label\">kv</span> <span id=\"__solidev_kv_count\" style=\"color:#b8e986;\">{kv_count}c</span>{kv_btn_extra}</button>\
<span style=\"color:#30363d;\">|</span>\
<button type=\"button\" id=\"__solidev_fb\" class=\"__solidev_mob\" title=\"click to expand the flamegraph (hierarchical timing per phase + per Soli function)\" style=\"padding:0 0.25rem;border-radius:0.25rem;color:#c9d1d9;font:inherit;cursor:pointer;border:none;background:transparent;\"><svg class=\"__solidev_icon\" width=\"14\" height=\"14\" viewBox=\"0 0 24 24\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"2\" stroke-linecap=\"round\" stroke-linejoin=\"round\"><path d=\"M8.5 14.5A2.5 2.5 0 0 0 11 12c0-1.38-.5-2-1-3-1.072-2.143-.224-4.054 2-6 .5 2.5 2 4.9 4 6.5 2 1.6 3 3.5 3 5.5a7 7 0 1 1-14 0c0-1.153.433-2.294 1-3a2.5 2.5 0 0 0 2.5 2.5z\"/></svg><span class=\"__solidev_label\">flame</span> <span id=\"__solidev_flame_count\" style=\"color:#b8e986;\">{flame_count}s</span></button>\
</div>\
</div>{requests_panel}<div id=\"__solidev_panels\">{warnings_panel}{breakdown_panel}{queries_panel}{http_panel}{kv_panel}{flame_panel}</div></aside>\
<button type=\"button\" id=\"__solidev_show\" aria-label=\"Show dev bar\" style=\"display:none;position:fixed;bottom:0.5rem;right:0.5rem;z-index:2147483646;font-family:'JetBrains Mono',ui-monospace,monospace;font-size:10px;padding:0.25rem 0.5rem;border-radius:0.25rem;background:#0b0d0f;color:#f0c674;border:1px solid #30363d;letter-spacing:0.05em;cursor:pointer;\">DEV{n1_minimized_badge}{warn_minimized_badge}</button>\
<script>(function(){{var __dups=document.querySelectorAll('aside#__solidev_bar');for(var __i=0;__i<__dups.length-1;__i++){{if(__dups[__i].parentNode)__dups[__i].parentNode.removeChild(__dups[__i]);}}var bar=document.getElementById('__solidev_bar');var open=document.getElementById('__solidev_show');if(!bar||!open)return;var origPad=document.body.style.paddingBottom;function syncPad(){{if(bar.style.display==='none'){{document.body.style.paddingBottom=origPad;return;}}document.body.style.paddingBottom=bar.offsetHeight+'px';}}function setHidden(h){{if(h){{bar.style.display='none';open.style.display='inline-flex';try{{sessionStorage.setItem('__solidev_hidden','1');}}catch(e){{}}}}else{{bar.style.display='';open.style.display='none';try{{sessionStorage.removeItem('__solidev_hidden');}}catch(e){{}}}}syncPad();}}var hidden=false;try{{hidden=sessionStorage.getItem('__solidev_hidden')==='1';}}catch(e){{}}setHidden(hidden);if(typeof ResizeObserver!=='undefined'){{try{{new ResizeObserver(syncPad).observe(bar);}}catch(e){{}}}}window.addEventListener('resize',syncPad);var c=document.getElementById('__solidev_close');if(c)c.addEventListener('click',function(){{setHidden(true);}});open.addEventListener('click',function(){{setHidden(false);}});var db=document.getElementById('__solidev_db');if(db){{db.addEventListener('click',function(){{var qp=document.getElementById('__solidev_queries');if(qp)qp.style.display=qp.style.display==='none'?'block':'none';}});}}var hb=document.getElementById('__solidev_hb');if(hb){{hb.addEventListener('click',function(){{var hp=document.getElementById('__solidev_http');if(hp)hp.style.display=hp.style.display==='none'?'block':'none';}});}}var kb=document.getElementById('__solidev_kb');if(kb){{kb.addEventListener('click',function(){{var kp=document.getElementById('__solidev_kv');if(kp)kp.style.display=kp.style.display==='none'?'block':'none';}});}}var rb=document.getElementById('__solidev_rb');if(rb){{rb.addEventListener('click',function(){{var rp=document.getElementById('__solidev_phases');if(rp)rp.style.display=rp.style.display==='none'?'block':'none';}});}}var mwt=document.getElementById('__solidev_mw_toggle');var mws=document.getElementById('__solidev_mw_subrows');var mwc=document.getElementById('__solidev_mw_chev');if(mwt&&mws){{mwt.addEventListener('click',function(){{var hidden=mws.style.display==='none';mws.style.display=hidden?'':'none';if(mwc)mwc.textContent=hidden?'▼':'▶';}});}}var vwt=document.getElementById('__solidev_view_toggle');var vws=document.getElementById('__solidev_view_subrows');var vwc=document.getElementById('__solidev_view_chev');if(vwt&&vws){{vwt.addEventListener('click',function(){{var hidden=vws.style.display==='none';vws.style.display=hidden?'':'none';if(vwc)vwc.textContent=hidden?'▼':'▶';}});}}var fb=document.getElementById('__solidev_fb');if(fb){{fb.addEventListener('click',function(){{var fp=document.getElementById('__solidev_flame');if(fp)fp.style.display=fp.style.display==='none'?'block':'none';}});}}var fchart=document.getElementById('__solidev_flame_chart');var flist=document.getElementById('__solidev_flame_list');if(fchart){{var totalUs=parseFloat(fchart.getAttribute('data-total'))||1;var rects=fchart.querySelectorAll('.__solidev_rect');function applyZoom(viewStart,viewW){{rects.forEach(function(r){{var s=parseFloat(r.getAttribute('data-start'));var w=parseFloat(r.getAttribute('data-w'));var rs=s-viewStart;var re=rs+w;if(re<=0||rs>=viewW){{r.style.display='none';return;}}r.style.display='';var cs=Math.max(0,rs);var ce=Math.min(viewW,re);r.style.left=(cs/viewW*100)+'%';r.style.width=Math.max(0.001,(ce-cs)/viewW*100)+'%';}});}}function highlightRect(rect,on){{if(!rect)return;rect.style.outline=on?'2px solid #ffffff':'';rect.style.outlineOffset=on?'-2px':'';}}function highlightRow(li,on){{if(!li)return;li.style.background=on?'#1c1f23':'';if(on)li.scrollIntoView({{block:'nearest',behavior:'smooth'}});}}rects.forEach(function(r){{r.addEventListener('click',function(ev){{ev.stopPropagation();applyZoom(parseFloat(r.getAttribute('data-start')),parseFloat(r.getAttribute('data-w')));}});r.addEventListener('mouseenter',function(){{var idx=r.getAttribute('data-idx');var li=flist?flist.querySelector('li[data-idx=\"'+idx+'\"]'):null;highlightRow(li,true);highlightRect(r,true);}});r.addEventListener('mouseleave',function(){{var idx=r.getAttribute('data-idx');var li=flist?flist.querySelector('li[data-idx=\"'+idx+'\"]'):null;highlightRow(li,false);highlightRect(r,false);}});}});fchart.addEventListener('dblclick',function(){{applyZoom(0,totalUs);}});if(flist){{flist.querySelectorAll('li[data-idx]').forEach(function(li){{li.addEventListener('mouseenter',function(){{var idx=li.getAttribute('data-idx');var rect=fchart.querySelector('.__solidev_rect[data-idx=\"'+idx+'\"]');highlightRow(li,true);highlightRect(rect,true);}});li.addEventListener('mouseleave',function(){{var idx=li.getAttribute('data-idx');var rect=fchart.querySelector('.__solidev_rect[data-idx=\"'+idx+'\"]');highlightRow(li,false);highlightRect(rect,false);}});li.addEventListener('click',function(){{applyZoom(parseFloat(li.getAttribute('data-start')),parseFloat(li.getAttribute('data-w')));}});}});}}}}var vrows=document.querySelectorAll('#__solidev_bar [data-solidev-view-idx]');if(vrows.length){{var ov=null,lbl=null,markerCache=null,autoScroll=false;function ensureOverlay(){{if(ov)return;ov=document.createElement('div');ov.id='__solidev_view_outline';ov.style.cssText='position:absolute;pointer-events:none;outline:2px solid #b8e986;outline-offset:-2px;background:rgba(184,233,134,0.12);z-index:2147483645;display:none;border-radius:2px;';document.body.appendChild(ov);lbl=document.createElement('div');lbl.style.cssText='position:absolute;pointer-events:none;font-family:JetBrains Mono,ui-monospace,monospace;font-size:10px;background:#0b0d0f;color:#b8e986;border:1px solid #b8e986;padding:1px 6px;border-radius:3px;z-index:2147483645;display:none;white-space:nowrap;';document.body.appendChild(lbl);}}function buildCache(){{if(markerCache)return markerCache;markerCache={{}};var w=document.createTreeWalker(document.body,NodeFilter.SHOW_COMMENT,null);var n;while(n=w.nextNode()){{var v=n.nodeValue||'';var m=v.match(/^solidev:(view|partial|layout):(start|end) id=(\\d+)/);if(!m)continue;var id=m[3];if(!markerCache[id])markerCache[id]={{}};markerCache[id][m[2]]=n;}}return markerCache;}}function ensureVisible(rect){{var barH=(bar&&bar.style.display!=='none')?bar.offsetHeight:0;var vh=window.innerHeight||document.documentElement.clientHeight;var visBottom=vh-barH;var pad=24;var needsUp=rect.top<pad;var needsDown=rect.top>visBottom-pad||(rect.bottom>visBottom&&rect.height<visBottom-2*pad);if(!needsUp&&!needsDown)return false;autoScroll=true;var sy=window.scrollY||window.pageYOffset||0;var targetY=sy+rect.top-Math.max(80,(visBottom-rect.height)/2);if(targetY<0)targetY=0;window.scrollTo({{top:targetY,left:window.scrollX||0,behavior:'auto'}});setTimeout(function(){{autoScroll=false;}},0);return true;}}function showFor(id,name){{var pair=buildCache()[id];if(!pair||!pair.start||!pair.end)return;var range=document.createRange();try{{range.setStartAfter(pair.start);range.setEndBefore(pair.end);}}catch(e){{return;}}var rect=range.getBoundingClientRect();if(rect.width===0&&rect.height===0)return;if(ensureVisible(rect)){{rect=range.getBoundingClientRect();}}ensureOverlay();var sx=window.scrollX||window.pageXOffset||0;var sy=window.scrollY||window.pageYOffset||0;ov.style.display='block';ov.style.left=(rect.left+sx)+'px';ov.style.top=(rect.top+sy)+'px';ov.style.width=rect.width+'px';ov.style.height=rect.height+'px';lbl.textContent=name;lbl.style.display='block';lbl.style.left=(rect.left+sx)+'px';lbl.style.top=Math.max(0,rect.top+sy-18)+'px';}}function hideOv(){{if(autoScroll)return;if(ov)ov.style.display='none';if(lbl)lbl.style.display='none';}}vrows.forEach(function(li){{li.addEventListener('mouseenter',function(){{var id=li.getAttribute('data-solidev-view-idx');var n=li.getAttribute('data-solidev-view-name');if(!n){{var nameEl=li.querySelector('span[title]');n=nameEl?nameEl.textContent:'';}}showFor(id,n);}});li.addEventListener('mouseleave',hideOv);}});}}{net_patch}if(!window.__solidevSwapHook){{window.__solidevSwapHook=1;var __clean=function(){{var bars=document.querySelectorAll('aside#__solidev_bar');for(var i=0;i<bars.length-1;i++){{if(bars[i].parentNode)bars[i].parentNode.removeChild(bars[i]);}}var b=bars[bars.length-1];if(b){{document.body.style.paddingBottom=(b.style.display==='none')?'':(b.offsetHeight+'px');}}}};document.addEventListener('htmx:afterSwap',__clean);document.addEventListener('soli:load',__clean);}}document.addEventListener('keydown',function(e){{if(e.altKey&&(e.key==='d'||e.key==='D')){{e.preventDefault();setHidden(bar.style.display!=='none');}}}});}})();</script>",
        marker = MARKER,
        request_id = html_escape(&ctx.request_id),
        method = html_escape(&ctx.method),
        path = html_escape(&ctx.path),
        status_color = status_color,
        status = status,
        route_arrow = route_arrow,
        requests_panel = requests_panel,
        net_patch = NET_PATCH,
        render = html_escape(&render_str),
        rss = html_escape(&rss_str),
        q_count = q_count,
        q_btn_extra = q_btn_extra,
        db_label_color = db_label_color,
        n1_minimized_badge = n1_minimized_badge,
        warnings_panel = warnings_panel,
        warn_minimized_badge = warn_minimized_badge,
        h_count = h_count,
        h_btn_extra = h_btn_extra,
        kv_count = kv_count,
        kv_btn_extra = kv_btn_extra,
        flame_count = flame_count,
        queries_panel = queries_panel,
        http_panel = http_panel,
        kv_panel = kv_panel,
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
        SpanKind::Component => "#79d9c0",
        SpanKind::Db => "#bd93f9",
        SpanKind::Http => "#ff79c6",
        SpanKind::Kv => "#ffb86c",
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
            "<li class=\"__solidev_flame_li\" data-idx=\"{idx}\" data-start=\"{ds}\" data-w=\"{dw}\"{view_attrs} style=\"display:flex;align-items:center;gap:0.5rem;padding:0.125rem 0.25rem;border-radius:0.125rem;cursor:zoom-in;\">\
<span style=\"flex:0 0 auto;width:0.5rem;height:0.5rem;background:{color};border-radius:0.125rem;display:inline-block;\"></span>\
<span class=\"__solidev_flame_kind\" style=\"flex:0 0 auto;width:5.5rem;font-size:9px;color:#8b949e;text-transform:uppercase;letter-spacing:0.05em;\">{kind}</span>\
<span style=\"flex:1;color:#e6e6e6;white-space:nowrap;overflow:hidden;text-overflow:ellipsis;padding-left:{indent}px;\">{name}{meta}</span>\
<span class=\"__solidev_flame_dur\" style=\"flex:0 0 auto;color:#b8e986;font-variant-numeric:tabular-nums;width:5rem;text-align:right;\">{dur_str}</span>\
<span class=\"__solidev_col_pct\" style=\"flex:0 0 auto;color:#8b949e;font-variant-numeric:tabular-nums;width:3.5rem;text-align:right;\">{pct:.1}%</span>\
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
        "<div id=\"__solidev_flame\" class=\"__solidev_panel\" style=\"display:none;border-top:1px solid #30363d;background:#08090b;padding:0.5rem 0.75rem;\">\
<div class=\"__solidev_flame_head\" style=\"display:flex;align-items:center;gap:0.75rem;margin-bottom:0.5rem;font-size:10px;color:#8b949e;letter-spacing:0.08em;\">\
<span>FLAMEGRAPH · {n_spans} SPAN{plural} · {total_str}</span>\
<span class=\"__solidev_flame_help\" style=\"color:#30363d;\">|</span>\
<span class=\"__solidev_flame_help\" style=\"color:#6c7280;\">click a span to zoom in · double-click the chart to reset</span>\
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
        "<li class=\"__solidev_pr\" style=\"display:flex;align-items:center;gap:0.75rem;\">\
<span class=\"__solidev_pr_name\" style=\"flex:0 0 5.5rem;color:{color};\">{name}</span>\
<span class=\"__solidev_pr_dur\" style=\"flex:0 0 4.5rem;color:#e6e6e6;font-variant-numeric:tabular-nums;text-align:right;\">{dur}</span>\
<span class=\"__solidev_col_pct\" style=\"flex:0 0 2.5rem;color:#8b949e;font-variant-numeric:tabular-nums;text-align:right;\">{pct}%</span>\
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
        "<li id=\"{toggle_id}\" class=\"__solidev_pr\" title=\"{title}\" style=\"display:flex;align-items:center;gap:0.75rem;cursor:pointer;user-select:none;\">\
<span class=\"__solidev_pr_name\" style=\"flex:0 0 5.5rem;color:{color};\"><span id=\"{chev_id}\" style=\"color:#8b949e;font-size:9px;margin-right:0.25rem;\">▶</span>{name}</span>\
<span class=\"__solidev_pr_dur\" style=\"flex:0 0 4.5rem;color:#e6e6e6;font-variant-numeric:tabular-nums;text-align:right;\">{dur}</span>\
<span class=\"__solidev_col_pct\" style=\"flex:0 0 2.5rem;color:#8b949e;font-variant-numeric:tabular-nums;text-align:right;\">{pct}%</span>\
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
<span class=\"__solidev_sub_glyph\" style=\"flex:0 0 4.5rem;color:#8b949e;font-size:10px;\">└─</span>\
<span class=\"__solidev_sub_name\" style=\"flex:0 0 9rem;color:#e6e6e6;overflow:hidden;text-overflow:ellipsis;\" title=\"{title}\">{name}</span>\
<span class=\"__solidev_sub_dur\" style=\"flex:0 0 4.5rem;color:#e6e6e6;font-variant-numeric:tabular-nums;text-align:right;\">{dur}</span>\
<span class=\"__solidev_col_pct\" style=\"flex:0 0 2.5rem;color:#8b949e;font-variant-numeric:tabular-nums;text-align:right;\">{pct}%</span>\
<span class=\"__solidev_sub_bar\" style=\"flex:1;height:0.375rem;background:#1c1f23;border-radius:0.125rem;overflow:hidden;\"><span style=\"display:block;width:{bar}%;height:100%;background:{color};opacity:0.7;\"></span></span>\
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
        "<li data-solidev-view-idx=\"{id}\" style=\"display:flex;align-items:center;gap:0.75rem;padding-left:{indent:.2}rem;cursor:pointer;\" title=\"hover to outline this template's region in the page\">\
<span style=\"flex:0 0 1.25rem;color:#8b949e;font-size:10px;\">{glyph}</span>\
<span style=\"flex:1 1 auto;color:#e6e6e6;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;\" title=\"{title}\">{name}</span>\
<span class=\"__solidev_view_sub_dur\" style=\"flex:0 0 4.5rem;color:#e6e6e6;font-variant-numeric:tabular-nums;text-align:right;\">{dur}</span>\
<span class=\"__solidev_col_pct\" style=\"flex:0 0 2.5rem;color:#8b949e;font-variant-numeric:tabular-nums;text-align:right;\">{pct}%</span>\
<span class=\"__solidev_view_sub_bar\" style=\"flex:0 0 8rem;height:0.375rem;background:#1c1f23;border-radius:0.125rem;overflow:hidden;\"><span style=\"display:block;width:{bar}%;height:100%;background:{color};opacity:0.7;\"></span></span>\
</li>",
        id = id,
        indent = base_indent_rem,
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
///
/// Returned tuples are `(template, count, total_duration_us)`. Reused by the
/// test runner (via the `x-soli-test-n1` header) so a spec's
/// `assert_no_n_plus_one` uses the exact same detection as the dev-bar badge.
pub(crate) fn detect_n_plus_one(
    queries: &[LoggedQuery],
    threshold: usize,
) -> Vec<(String, usize, u64)> {
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

pub(crate) fn html_escape(s: &str) -> String {
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

    fn ctx(method: &str, path: &str) -> DevBarContext {
        DevBarContext {
            method: method.to_string(),
            path: path.to_string(),
            status: 200,
            elapsed_us: 1234,
            request_id: "test-req-id".to_string(),
            route: Some("home#index".to_string()),
            queries: vec![],
            http_requests: vec![],
            phases: vec![],
            middlewares: vec![],
            views: vec![],
            spans: vec![],
            kv_calls: vec![],
            warnings: vec![],
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
        // minimized "DEV" pill should carry an N+1 red badge
        let pill_pos = out.find("__solidev_show").expect("minimized pill present");
        let pill_end = out[pill_pos..].find("</button>").expect("pill closes") + pill_pos;
        let pill = &out[pill_pos..pill_end];
        assert!(
            pill.contains("N+1"),
            "minimized pill should show N+1 badge: {pill}"
        );
        assert!(pill.contains("#ff6b6b"), "N+1 badge should be red: {pill}");
    }

    #[test]
    fn minimized_pill_has_no_n_plus_one_badge_when_clean() {
        let html = "<html><body></body></html>";
        let mut c = ctx("GET", "/orders");
        // A single distinct query — no N+1.
        c.queries
            .push(q("FOR doc IN orders FILTER doc.id == @id RETURN doc", 0.1));
        let out = inject_dev_bar(html, &c);
        let pill_pos = out.find("__solidev_show").expect("minimized pill present");
        let pill_end = out[pill_pos..].find("</button>").expect("pill closes") + pill_pos;
        let pill = &out[pill_pos..pill_end];
        assert!(
            !pill.contains("N+1"),
            "minimized pill should not show N+1 badge: {pill}"
        );
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
    fn view_aggregate_uses_root_spans_not_phase_marker() {
        let html = "<html><body></body></html>";
        let mut c = ctx("GET", "/campfire");
        // The bug scenario: the "view" phase marker is absent on this render
        // path, so the old code summed nothing and showed the aggregate as
        // 0ms even though the per-template spans had real time. Nested spans:
        // comment_thread inside comment_widget inside the page (one root).
        c.views
            .push((2, Some(1), "shared/comment_thread".into(), 16_500));
        c.views
            .push((1, Some(0), "shared/comment_widget".into(), 17_400));
        c.views.push((0, None, "campfire/index".into(), 21_800));
        let out = inject_dev_bar(html, &c);
        // Aggregate view total = the single root (21.8ms), NOT 0ms. The value
        // now appears twice: once in the aggregate row, once in the root
        // sub-row. Before the fix the aggregate showed "0µs" so it appeared
        // only once (the sub-row).
        assert_eq!(out.matches("21.8ms").count(), 2);
        // Roots are not double-counted by their nested children.
        assert_eq!(out.matches("17.4ms").count(), 1);
        assert_eq!(out.matches("16.5ms").count(), 1);
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
        // Button still renders but with `0s`.
        assert!(out.contains("0s</span>"));
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

    #[test]
    fn injects_bar_when_marker_absent() {
        let html = "<html><body><p>page</p></body></html>";
        let out = inject_dev_bar(html, &ctx("GET", "/"));
        assert_ne!(out, html);
        assert!(out.contains(MARKER));
        assert!(out.contains("id=\"__solidev_bar\""));
    }

    #[test]
    fn noop_when_marker_already_present() {
        let html = format!("<html><body><p>x</p><!-- {} --></body></html>", MARKER);
        let out = inject_dev_bar(&html, &ctx("GET", "/"));
        assert_eq!(out, html);
        assert_eq!(out.matches(MARKER).count(), 1);
    }

    #[test]
    fn htmx_request_is_detected() {
        assert!(!is_htmx_request(None));
        assert!(is_htmx_request(Some("true")));
        assert!(is_htmx_request(Some("TRUE")));
        assert!(!is_htmx_request(Some("false")));
    }

    #[test]
    fn htmx_partial_response_skips_dev_bar_injection() {
        // Mirrors the gate in `handle_request`: an HX-Request fragment must
        // not be rewritten, because the host page already carries the bar.
        let html = "<html><body><p>fragment</p></body></html>";

        let body = if is_htmx_request(Some("true")) {
            html.to_string()
        } else {
            inject_dev_bar(html, &ctx("GET", "/users"))
        };
        assert!(!body.contains(MARKER));
        assert!(!body.contains("__solidev_bar"));
    }
}
