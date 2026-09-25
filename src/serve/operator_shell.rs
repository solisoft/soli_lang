//! The one look every built-in `/__soli/*` page shares: errors, slow queries, jobs, the mail
//! inbox, and the mailer and component catalogs.
//!
//! Each page used to carry its own copy of a dark stylesheet, and the copies
//! had drifted — different paddings, different buttons, a different header on
//! each. [`page`] is the document they all render into now: a top bar that
//! moves between the pages, a heading, and one stylesheet whose classes the
//! pages share (`.tabs`, `.toolbar`, `.table-wrap`, `.gallery`, `.empty`,
//! `.notice`, the status colours).
//!
//! The pages that exist only under `--dev` (inbox, mailers, components) are
//! left out of the bar in production, where they answer 404.

use super::dev_bar::html_escape;

/// Which page is rendering, for the bar's current item.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Section {
    Errors,
    SlowQueries,
    Jobs,
    Inbox,
    Mailers,
    Components,
}

impl Section {
    fn href(self) -> &'static str {
        match self {
            Section::Errors => "/__soli/errors",
            Section::SlowQueries => "/__soli/slow_queries",
            Section::Jobs => "/__soli/jobs",
            Section::Inbox => "/__soli/inbox",
            Section::Mailers => "/__soli/mailers",
            Section::Components => "/__soli/components",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Section::Errors => "Errors",
            Section::SlowQueries => "Slow queries",
            Section::Jobs => "Jobs",
            Section::Inbox => "Inbox",
            Section::Mailers => "Mailers",
            Section::Components => "Components",
        }
    }

    fn dev_only(self) -> bool {
        matches!(
            self,
            Section::Inbox | Section::Mailers | Section::Components
        )
    }
}

const SECTIONS: [Section; 6] = [
    Section::Errors,
    Section::SlowQueries,
    Section::Jobs,
    Section::Inbox,
    Section::Mailers,
    Section::Components,
];

/// A whole page: `title` is the heading, `intro` the line under it (HTML,
/// may be empty), `body` everything else.
pub(crate) fn page(section: Section, title: &str, intro: &str, body: &str) -> String {
    page_with(
        section,
        title,
        intro,
        body,
        crate::interpreter::builtins::template::is_dev_mode(),
    )
}

fn page_with(section: Section, title: &str, intro: &str, body: &str, dev_mode: bool) -> String {
    let mut nav = String::new();
    for item in SECTIONS {
        if item.dev_only() && !dev_mode {
            continue;
        }
        let current = if item == section {
            " aria-current=\"page\""
        } else {
            ""
        };
        nav.push_str(&format!(
            "<a href=\"{}\"{current}>{}</a>",
            item.href(),
            item.label()
        ));
    }
    let intro = if intro.is_empty() {
        String::new()
    } else {
        format!("<p class=\"intro\">{intro}</p>")
    };
    format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">\
<meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">\
<meta name=\"robots\" content=\"noindex\">\
<title>{title_text} \u{b7} Soli</title><style>{STYLE}</style></head><body>\
<header class=\"top\"><div class=\"top-in\">\
<a class=\"brand\" href=\"{home}\"><span class=\"mark\" aria-hidden=\"true\"></span>soli</a>\
<nav class=\"sections\" aria-label=\"Soli pages\">{nav}</nav>\
<a class=\"to-app\" href=\"/\">Back to app <span aria-hidden=\"true\">\u{2197}</span></a>\
</div></header>\
<main class=\"wrap\"><div class=\"head\"><h1>{title_text}</h1>{intro}</div>{body}</main>\
</body></html>",
        title_text = html_escape(title),
        home = section.href(),
    )
}

/// The stylesheet. Dark on purpose: these pages sit beside the dev bar.
const STYLE: &str = r#"
:root{
  --bg:#0b0d10;--panel:#11141a;--panel-2:#161a21;--sunken:#080a0d;
  --line:#232933;--line-2:#323a46;
  --text:#e6e9ee;--text-2:#b3bbc7;--muted:#7f8896;--faint:#5b6371;
  --link:#79c0ff;--primary:#2f81f7;--primary-2:#1f6feb;
  --red:#ff7b72;--red-bg:#2a1214;--amber:#e3b341;--amber-bg:#261e0c;
  --green:#7ee787;--green-bg:#0f2417;--blue:#79c0ff;
  --sans:ui-sans-serif,system-ui,-apple-system,"Segoe UI",Roboto,"Helvetica Neue",Arial,sans-serif;
  --mono:"JetBrains Mono",ui-monospace,SFMono-Regular,Menlo,Consolas,monospace;
  color-scheme:dark;
}
*{box-sizing:border-box}
html{-webkit-text-size-adjust:100%}
body{margin:0;background:var(--bg);color:var(--text);font:14px/1.5 var(--sans)}
a{color:var(--link);text-decoration:none}
a:hover{text-decoration:underline}
:focus-visible{outline:2px solid var(--link);outline-offset:2px;border-radius:4px}
code,pre,kbd{font-family:var(--mono);font-size:12px}
code{background:var(--panel-2);border:1px solid var(--line);border-radius:4px;padding:0 .3em}

.top{position:sticky;top:0;z-index:5;background:rgba(11,13,16,.88);backdrop-filter:blur(8px);border-bottom:1px solid var(--line)}
.top-in{max-width:1200px;margin:0 auto;padding:0 24px;height:52px;display:flex;align-items:center;gap:24px}
.brand{display:flex;align-items:center;gap:8px;color:var(--text);font:600 15px/1 var(--sans);letter-spacing:-.01em}
.brand:hover{text-decoration:none}
.mark{width:14px;height:14px;border-radius:4px;background:linear-gradient(135deg,#ff7b72,#e3b341)}
.sections{display:flex;gap:2px;overflow-x:auto;scrollbar-width:none;min-width:0;flex:1}
.sections a{color:var(--muted);padding:6px 10px;border-radius:6px;font-size:13px;white-space:nowrap}
.sections a:hover{color:var(--text);background:var(--panel-2);text-decoration:none}
.sections a[aria-current=page]{color:var(--text);background:var(--panel-2);box-shadow:inset 0 0 0 1px var(--line-2)}
.to-app{color:var(--muted);font-size:13px;white-space:nowrap}
.to-app:hover{color:var(--text);text-decoration:none}

.wrap{max-width:1200px;margin:0 auto;padding:28px 24px 64px}
.head{margin:0 0 20px;display:grid;gap:6px}
h1{margin:0;font-size:22px;line-height:1.25;font-weight:600;letter-spacing:-.015em;text-wrap:balance}
h2{margin:0;font-size:16px;line-height:1.4;font-weight:600;text-wrap:balance}
h3{margin:0 0 8px;font-size:12px;font-weight:600;color:var(--muted);text-transform:uppercase;letter-spacing:.06em}
.intro{margin:0;color:var(--muted);max-width:72ch}
p{margin:0 0 12px}
.muted{color:var(--muted);font-size:12px}
.err{color:var(--red)}
.mono{font-family:var(--mono);font-size:12px}
.num{font-variant-numeric:tabular-nums}

button,.btn{display:inline-flex;align-items:center;gap:6px;height:30px;padding:0 12px;border-radius:6px;border:1px solid transparent;
  background:var(--primary-2);color:#fff;font:500 13px/1 var(--sans);cursor:pointer;white-space:nowrap}
button:hover,.btn:hover{background:var(--primary);text-decoration:none}
button.ghost,.btn.ghost{background:transparent;color:var(--text-2);border-color:var(--line-2)}
button.ghost:hover,.btn.ghost:hover{background:var(--panel-2);color:var(--text)}
button.danger{background:transparent;color:var(--red);border-color:#5a2a2a}
button.danger:hover{background:var(--red-bg)}
.actions button,.row button,td button{height:26px;padding:0 9px;font-size:12px}
input,select,textarea{height:32px;background:var(--sunken);color:var(--text);border:1px solid var(--line-2);border-radius:6px;padding:0 10px;font:13px var(--sans)}
input::placeholder{color:var(--faint)}
input:focus,select:focus{outline:none;border-color:var(--link);box-shadow:0 0 0 3px rgba(121,192,255,.15)}
form{margin:0}

.bar,.toolbar{display:flex;flex-wrap:wrap;align-items:center;gap:8px;margin:0 0 16px}
.grow{flex:1 1 auto}
.tabs{display:flex;gap:4px;border-bottom:1px solid var(--line);margin:0 0 16px;overflow-x:auto}
.tab{display:flex;align-items:center;gap:8px;padding:8px 12px;margin-bottom:-1px;color:var(--muted);font-size:13px;border-bottom:2px solid transparent;white-space:nowrap}
.tab:hover{color:var(--text);text-decoration:none}
.tab.on{color:var(--text);border-bottom-color:var(--red)}
.tab .n{font:11px var(--mono);color:var(--muted);background:var(--panel-2);border:1px solid var(--line);border-radius:999px;padding:1px 7px}
.tab.on .n{color:var(--text);border-color:var(--line-2)}
.summary{color:var(--muted);font-size:12px;margin:0 0 10px}

.notice{border:1px solid #4d3d17;background:var(--amber-bg);color:var(--amber);border-radius:8px;padding:10px 14px;font-size:13px;margin:0 0 16px}
.notice.bad{border-color:#5a2a2a;background:var(--red-bg);color:var(--red)}
.empty{border:1px dashed var(--line-2);border-radius:10px;padding:40px 16px;text-align:center;display:grid;gap:6px;justify-items:center}
.empty b{font-size:14px;font-weight:600}
.empty span{color:var(--muted);font-size:13px;max-width:52ch}

.tag{display:inline-flex;align-items:center;height:20px;padding:0 8px;border-radius:999px;border:1px solid var(--line-2);font:500 11px/1 var(--sans);color:var(--text-2);white-space:nowrap}
.pending,.scheduled,.captured,.regressed{color:var(--amber)}
.running{color:var(--blue)}
.failed,.dead,.open{color:var(--red)}
.done,.sent,.resolved{color:var(--green)}
.ignored{color:var(--muted)}
.tag.pending,.tag.scheduled,.tag.captured,.tag.regressed{border-color:#4d3d17;background:var(--amber-bg)}
.tag.failed,.tag.dead,.tag.open{border-color:#5a2a2a;background:var(--red-bg)}
.tag.done,.tag.sent,.tag.resolved{border-color:#1d4a2c;background:var(--green-bg)}
.tag.running{border-color:#1d3a5a;background:#0c1a2a}

.table-wrap{border:1px solid var(--line);border-radius:10px;overflow-x:auto;background:var(--panel)}
table{border-collapse:collapse;width:100%;font-size:13px}
th,td{padding:10px 14px;text-align:left;vertical-align:middle;border-bottom:1px solid var(--line)}
td .actions,td form{display:inline-flex;gap:6px}
tbody tr:last-child td{border-bottom:0}
th{font:600 11px/1.2 var(--sans);color:var(--muted);text-transform:uppercase;letter-spacing:.06em;background:var(--panel-2);white-space:nowrap}
tbody tr:hover td{background:rgba(255,255,255,.02)}
td.subj,.subj{color:var(--text)}
td .muted{font-size:12px}

pre{background:var(--sunken);border:1px solid var(--line);border-radius:8px;padding:12px 14px;overflow:auto;white-space:pre-wrap;word-break:break-word;max-height:60vh;margin:4px 0 14px;line-height:1.55;color:var(--text-2)}
details{border:1px solid var(--line);border-radius:10px;background:var(--panel);padding:0 14px;margin:0 0 10px}
details[open]{padding-bottom:6px}
summary{cursor:pointer;padding:12px 0;font-size:13px;color:var(--text-2);list-style-position:outside}
summary:hover{color:var(--text)}
dl{display:grid;grid-template-columns:max-content 1fr;gap:8px 20px;font-size:13px;margin:0 0 20px}
dt{color:var(--muted)}
dd{margin:0;word-break:break-word}
.panel{border:1px solid var(--line);border-radius:10px;background:var(--panel);padding:16px 18px;margin:0 0 16px}

.gallery{display:grid;grid-template-columns:repeat(auto-fill,minmax(300px,1fr));gap:16px}
.gallery.wide{grid-template-columns:repeat(auto-fill,minmax(380px,1fr))}
.card{border:1px solid var(--line);border-radius:10px;overflow:hidden;background:var(--panel);display:flex;flex-direction:column}
.card-head{padding:10px 14px;border-bottom:1px solid var(--line);display:grid;gap:2px}
.card-head a{color:var(--text);font:500 13px var(--mono)}
.card-head .muted{font-size:11px}
.card iframe{display:block;width:100%;border:0;background:#fff}
iframe.mail{width:100%;height:60vh;border:1px solid var(--line);border-radius:10px;background:#fff}

.pager{display:flex;gap:12px;align-items:center;color:var(--muted);font-size:12px;margin-top:14px}

/* the errors list */
.list{border:1px solid var(--line);border-radius:10px;overflow:hidden;background:var(--panel)}
.row{display:grid;grid-template-columns:48px minmax(0,1fr) 96px 110px auto;gap:18px;align-items:center;padding:14px 18px;border-top:1px solid var(--line)}
.row:first-child{border-top:0}
.row:hover{background:rgba(255,255,255,.02)}
.count{font:600 18px/1 var(--mono);text-align:left;font-variant-numeric:tabular-nums;color:var(--red)}
.row.resolved .count{color:var(--green)}
.row.ignored .count{color:var(--faint)}
.main{min-width:0;display:grid;gap:5px}
.msg{color:var(--text);font-size:14px;font-weight:500;line-height:1.4;display:-webkit-box;-webkit-line-clamp:2;-webkit-box-orient:vertical;overflow:hidden;word-break:break-word}
.msg:hover{color:var(--link);text-decoration:none}
.meta{display:flex;flex-wrap:wrap;gap:4px 12px;align-items:center;font:12px var(--mono);color:var(--muted);min-width:0}
.loc{overflow:hidden;text-overflow:ellipsis;white-space:nowrap;max-width:100%}
.req{color:var(--text-2)}
.trend svg{display:block;fill:var(--red)}
.trend .z{fill:var(--line-2)}
.row.resolved .trend svg{fill:var(--green)}
.row.ignored .trend svg{fill:var(--faint)}
.when{display:grid;gap:3px;font-size:13px;color:var(--text-2);text-align:right;white-space:nowrap}
.when .first{font-size:11px;color:var(--faint)}
.actions{display:flex;gap:6px;justify-content:flex-end;flex-wrap:wrap}

/* the slow-queries list: a query shape where an error has its message */
.row.slow .count{color:var(--amber)}
.row.slow .trend svg{fill:var(--amber)}
.msg.query{font:13px/1.5 var(--mono);color:var(--text-2)}
/* A query is read symbol by symbol: `->>` must not turn into an arrow. */
pre,code,.mono,.meta,.msg.query{font-variant-ligatures:none}
.ms{color:var(--text)}

@media (max-width:760px){
  .top-in{padding:0 16px;gap:14px}
  .to-app{display:none}
  .wrap{padding:20px 16px 48px}
  .row{grid-template-columns:36px minmax(0,1fr);gap:8px 12px;padding:12px 14px}
  .count{font-size:15px;align-self:start}
  .trend,.when,.actions{grid-column:2}
  .when{text-align:left;grid-auto-flow:column;justify-content:start;gap:12px}
  .actions{justify-content:flex-start}
  dl{grid-template-columns:1fr;gap:2px}
  dd{margin-bottom:8px}
}
@media (prefers-reduced-motion:reduce){*{transition:none!important;scroll-behavior:auto!important}}
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn production_hides_the_dev_only_pages() {
        let html = page_with(Section::Errors, "Errors", "", "", false);
        assert!(html.contains("href=\"/__soli/jobs\""));
        assert!(!html.contains("href=\"/__soli/inbox\""));
        assert!(!html.contains("href=\"/__soli/components\""));
        let dev = page_with(Section::Errors, "Errors", "", "", true);
        assert!(dev.contains("href=\"/__soli/inbox\""));
    }

    #[test]
    fn the_current_page_is_marked_and_the_title_escaped() {
        let html = page_with(Section::Jobs, "<Jobs>", "", "", true);
        assert!(html.contains("<a href=\"/__soli/jobs\" aria-current=\"page\">Jobs</a>"));
        assert!(html.contains("&lt;Jobs&gt;"));
        assert!(!html.contains("<Jobs>"));
    }
}
