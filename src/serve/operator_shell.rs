//! The one look every built-in `/__soli/*` page shares: the overview, errors,
//! slow queries, jobs, mobile, the mail inbox, and the mailer and component
//! catalogs.
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
    Overview,
    Errors,
    SlowQueries,
    Spam,
    Jobs,
    Mobile,
    Inbox,
    Mailers,
    Components,
}

impl Section {
    fn href(self) -> &'static str {
        match self {
            Section::Overview => "/__soli/",
            Section::Errors => "/__soli/errors",
            Section::SlowQueries => "/__soli/slow_queries",
            Section::Spam => "/__soli/spam",
            Section::Jobs => "/__soli/jobs",
            Section::Mobile => "/__soli/mobile",
            Section::Inbox => "/__soli/inbox",
            Section::Mailers => "/__soli/mailers",
            Section::Components => "/__soli/components",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Section::Overview => "Overview",
            Section::Errors => "Errors",
            Section::SlowQueries => "Queries",
            Section::Spam => "Spam",
            Section::Jobs => "Jobs",
            Section::Mobile => "Mobile",
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

const SECTIONS: [Section; 9] = [
    Section::Overview,
    Section::Errors,
    Section::SlowQueries,
    Section::Spam,
    Section::Jobs,
    Section::Mobile,
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

/// What a page adds to the shell beyond its heading and body.
#[derive(Default)]
pub(crate) struct Extras<'a> {
    /// Trusted markup for `<head>` (a `<meta http-equiv="refresh">`).
    pub head: &'a str,
    /// Trusted markup beside the heading (the overview's refresh controls).
    pub aside: &'a str,
}

fn page_with(section: Section, title: &str, intro: &str, body: &str, dev_mode: bool) -> String {
    render(section, title, intro, body, &Extras::default(), dev_mode)
}

pub(crate) fn render(
    section: Section,
    title: &str,
    intro: &str,
    body: &str,
    extras: &Extras<'_>,
    dev_mode: bool,
) -> String {
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
    let head = if extras.aside.is_empty() {
        format!(
            "<div class=\"head\"><h1>{}</h1>{intro}</div>",
            html_escape(title)
        )
    } else {
        format!(
            "<div class=\"head split\"><div class=\"head-text\"><h1>{}</h1>{intro}</div>\
<div class=\"head-aside\">{}</div></div>",
            html_escape(title),
            extras.aside
        )
    };
    format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">\
<meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">\
<meta name=\"robots\" content=\"noindex\">\
<title>{title_text} \u{b7} Soli</title>{head_extra}<style>{STYLE}</style></head><body>\
<header class=\"top\"><div class=\"top-in\">\
<a class=\"brand\" href=\"{home}\"><span class=\"mark\" aria-hidden=\"true\"></span>soli</a>\
<nav class=\"sections\" aria-label=\"Soli pages\">{nav}</nav>\
<a class=\"to-app\" href=\"/\">Back to app <span aria-hidden=\"true\">\u{2197}</span></a>\
</div></header>\
<main class=\"wrap\">{head}{body}</main>\
</body></html>",
        title_text = html_escape(title),
        head_extra = extras.head,
        // The brand goes to the overview, which opens for anyone who can
        // open any page here.
        home = Section::Overview.href(),
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
[hidden]{display:none!important}
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
details.hit{border-color:var(--amber)}
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
.qr-card{margin:0}
.qr-card svg{display:block;margin:14px auto 0;border-radius:6px}
.qr-card figcaption{padding:10px 14px 14px;text-align:center;font-size:13px}
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

/* the spam list: the learnt score where an error has its count */
.count.score{font-size:16px}
.row.spam-mid .count{color:var(--amber)}
.row.spam-none .count{color:var(--faint)}
.row.spam-lo .count{color:var(--green)}
.trend .tag{cursor:help}

/* the overview */
.head.split{display:flex;flex-wrap:wrap;align-items:flex-end;justify-content:space-between;gap:12px}
.head-text{display:grid;gap:6px}
.head-aside{display:flex;flex-wrap:wrap;align-items:center;gap:10px;color:var(--muted);font-size:13px}
.head-aside .btn{height:32px}
.ov{display:flex;flex-direction:column;gap:28px}
.tiles{display:grid;grid-template-columns:repeat(auto-fit,minmax(170px,1fr));gap:12px}
.tile{background:var(--panel);border:1px solid var(--line);border-radius:10px;padding:14px 16px;display:flex;flex-direction:column;gap:6px;color:var(--text);min-width:0}
.tile:hover{text-decoration:none;border-color:var(--line-2);background:var(--panel-2)}
.tile-label{color:var(--text-2);font-size:12px;font-weight:600;text-transform:uppercase;letter-spacing:.04em}
.tile-value{font-size:26px;line-height:1.2;font-weight:700;font-variant-numeric:tabular-nums;letter-spacing:-.01em;overflow-wrap:anywhere}
.tile-note{font-size:12px;display:flex;align-items:center;gap:6px;color:var(--text-2)}
.dot{width:8px;height:8px;border-radius:50%;display:inline-block;flex:0 0 auto;background:var(--muted)}
.dot.red{background:var(--red)}.dot.amber{background:var(--amber)}.dot.green{background:var(--green)}
.attn{background:var(--panel);border:1px solid var(--line);border-radius:12px;overflow:hidden}
.attn-head{display:flex;flex-wrap:wrap;align-items:center;justify-content:space-between;gap:8px;padding:16px 20px;border-bottom:1px solid var(--line)}
.attn-head h2{font-size:17px;font-weight:700}
.counts{display:flex;flex-wrap:wrap;gap:8px;font-size:12px}
.pill{display:inline-flex;align-items:center;border-radius:999px;padding:3px 10px;font-weight:600}
.crit{background:var(--red-bg);color:var(--red)}
.warn{background:var(--amber-bg);color:var(--amber)}
.info{background:#0f1d2e;color:var(--blue)}
.attn ol{list-style:none;margin:0;padding:0}
.item{display:flex;flex-wrap:wrap;align-items:center;gap:12px 20px;padding:14px 20px;border-bottom:1px solid #1a1f27}
.badge{flex:0 0 auto;width:76px;text-align:center;font-size:11px;font-weight:700;text-transform:uppercase;letter-spacing:.05em;border-radius:6px;padding:5px 0}
.item-main{flex:999 1 420px;min-width:0;display:flex;flex-direction:column;gap:4px}
.item-title{display:flex;flex-wrap:wrap;align-items:baseline;gap:8px}
.item-title b{font-weight:600;overflow-wrap:anywhere}
.chip{font-size:12px;color:var(--muted);border:1px solid var(--line-2);border-radius:4px;padding:0 6px}
.item-detail{font:12.5px/1.45 var(--mono);color:var(--text-2);overflow-wrap:anywhere;font-variant-ligatures:none}
.item-why{font-size:12.5px;color:var(--muted)}
.item-metric{flex:1 1 140px;display:flex;flex-direction:column;align-items:flex-end;gap:2px;text-align:right}
.item-metric b{font-size:18px;font-weight:700;font-variant-numeric:tabular-nums}
.item-metric span{font-size:12px;color:var(--text-2)}
.item .btn,.item button{height:36px;padding:0 12px;font-size:13px}
.attn-empty{padding:28px 20px;display:grid;gap:4px;border-bottom:1px solid #1a1f27}
.attn-empty b{font-size:15px}
.attn-empty span{color:var(--muted)}
.attn-foot{padding:12px 20px;font-size:13px;color:var(--muted)}
.usage{display:flex;flex-direction:column;gap:12px}
.usage-head{display:flex;flex-wrap:wrap;align-items:baseline;justify-content:space-between;gap:8px}
.usage-head h2{font-size:17px;font-weight:700}
.range{display:flex;gap:4px;background:var(--panel);border:1px solid var(--line);border-radius:8px;padding:3px}
.range a{color:var(--text-2);border-radius:6px;padding:6px 12px;min-height:32px;display:inline-flex;align-items:center}
.range a:hover{text-decoration:none;color:var(--text)}
.range a[aria-current]{background:var(--panel-2);color:var(--text);font-weight:600}
.usage-grid{display:flex;flex-wrap:wrap;gap:12px}
.ucard{flex:999 1 560px;min-width:0;background:var(--panel);border:1px solid var(--line);border-radius:12px;padding:18px 20px;display:flex;flex-direction:column;gap:14px}
.ucard.routes{flex:1 1 380px;padding:18px 0 8px;gap:10px}
.ucard-head{display:flex;flex-wrap:wrap;justify-content:space-between;gap:6px}
.ucard.routes .ucard-head{padding:0 20px}
.ucard h3{margin:0;font-size:14px;font-weight:600;color:var(--text);text-transform:none;letter-spacing:0}
.ucard-head span{font-size:12px;color:var(--muted)}
.chart{display:flex;gap:8px}
.yaxis{display:flex;flex-direction:column;justify-content:space-between;height:180px;font-size:11px;color:var(--muted);text-align:right;font-variant-numeric:tabular-nums}
.cols{flex:1 1 auto;min-width:0;height:180px;display:flex;align-items:flex-end;gap:2px;border-bottom:1px solid var(--line-2);background-image:linear-gradient(#1a1f27 1px,transparent 1px);background-size:100% 60px}
.cols .c{flex:1 1 0;min-width:0;background:var(--primary);border-radius:3px 3px 0 0;opacity:.9}
.cols .c.now{background:#a5d6ff}
.cols .c:hover{opacity:1;outline:2px solid var(--text);outline-offset:1px}
.xaxis{display:flex;justify-content:space-between;padding-left:32px;font-size:11px;color:var(--muted);font-variant-numeric:tabular-nums}
.status-mix{display:flex;flex-direction:column;gap:8px;padding-top:6px;border-top:1px solid #1a1f27}
.seg{display:flex;gap:2px;height:14px;border-radius:4px;overflow:hidden;background:var(--sunken)}
.seg span{min-width:2px}
.s2{background:var(--green)}.s3{background:var(--blue)}.s4{background:var(--amber)}.s5{background:var(--red)}
.legend{display:flex;flex-wrap:wrap;gap:6px 18px;font-size:12px;color:var(--text-2);font-variant-numeric:tabular-nums}
.legend span{display:inline-flex;align-items:center;gap:6px}
.legend i{width:10px;height:10px;border-radius:3px;display:inline-block}
.ucard .scroll{overflow-x:auto}
.rt{font-variant-numeric:tabular-nums}
.rt th,.rt td{background:transparent;padding:8px;text-align:right;border-bottom:0;border-top:1px solid #1a1f27}
.rt thead th{border-top:0;font-size:11px;padding:6px 8px}
.rt th:first-child,.rt td:first-child{text-align:left;padding-left:20px}
.rt th:last-child,.rt td:last-child{padding-right:20px}
.rt .route{display:flex;flex-direction:column;gap:3px;min-width:0}
.rt .route code{background:none;border:0;padding:0;font-size:12.5px;white-space:nowrap;color:var(--text)}
.share{height:3px;border-radius:2px;background:#1a1f27;display:block}
.share span{display:block;height:3px;border-radius:2px;background:var(--primary)}
.rt .slow{color:var(--amber);font-weight:600}
.rt .bad{color:var(--red);font-weight:600}
.rt .ok{color:var(--text-2)}
.usage-empty{color:var(--muted);font-size:13px;padding:24px 0;text-align:center}

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
  .item{padding:14px 16px}
  .item-metric{align-items:flex-start;text-align:left}
  .attn-head,.attn-foot,.attn-empty{padding-left:16px;padding-right:16px}
  .ucard{padding:16px}
  .yaxis{display:none}
  .xaxis{padding-left:0}
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
        // The overview is the bar's first item, in production too.
        let prod = page_with(Section::Errors, "Errors", "", "", false);
        let nav = prod
            .split("aria-label=\"Soli pages\">")
            .nth(1)
            .unwrap_or("");
        assert!(
            nav.starts_with("<a href=\"/__soli/\">Overview</a>"),
            "{nav}"
        );
        assert!(html.contains("&lt;Jobs&gt;"));
        assert!(!html.contains("<Jobs>"));
    }
}
