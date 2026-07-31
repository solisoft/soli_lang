//! Markdown pages.
//!
//! Same converter options and the same SEC-022 URL policy as the `.md` view
//! pipeline ([`crate::template::markdown_to_html_safe_urls`]), with one
//! addition: fenced `soli` blocks are highlighted server-side by re-lexing
//! them with the language's own lexer
//! ([`crate::coverage::reporter::html_highlight_soli`]).
//!
//! Only `soli` is coloured. Guessing at the other languages with a heuristic
//! would produce confidently wrong highlighting, and a CDN highlighter would
//! break the offline promise — so every other fence renders as plain
//! monospace, which is honest about what the server knows.

use std::path::Path;

use hyper::{Response, StatusCode};
use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd};

use crate::coverage::reporter::html_highlight_soli;
use crate::template::renderer::html_escape;
use crate::template::safe_markdown_url;

use super::{html_page, shell, ResponseBody};

/// Fence info strings rendered with the Soli lexer.
const SOLI_LANGUAGES: &[&str] = &["soli", "sl"];

pub(crate) fn page(root: &Path, file: &Path, rel: &str, dev_mode: bool) -> Response<ResponseBody> {
    let source = match std::fs::read_to_string(file) {
        Ok(s) => s,
        Err(_) => {
            return html_page(
                shell::render(
                    root,
                    shell::Page {
                        title: format!("Unreadable · {}", rel),
                        current: rel,
                        meta: String::new(),
                        body: "<div class=\"state\"><p class=\"lead\">This file could not be read.</p></div>"
                            .to_string(),
                        prose: false,
            outline: String::new(),
                    },
                    dev_mode,
                ),
                StatusCode::INTERNAL_SERVER_ERROR,
            )
        }
    };

    let title = first_heading(&source).unwrap_or_else(|| {
        file.file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| rel.to_string())
    });

    let meta = std::fs::metadata(file)
        .map(|m| {
            let size = super::human_size(m.len());
            match super::human_age(m.modified().ok()) {
                age if age.is_empty() => size,
                age => format!("{} · edited {} ago", size, age),
            }
        })
        .unwrap_or_default();

    let (body, outline) = to_html_with_outline(&source);

    let html = shell::render(
        root,
        shell::Page {
            title,
            current: rel,
            meta,
            body,
            prose: true,
            outline: render_outline(&outline),
        },
        dev_mode,
    );
    html_page(html, StatusCode::OK)
}

/// One entry in a document's outline.
pub(crate) struct Heading {
    /// 2 or 3 — `h1` is the page title and lives in the masthead.
    pub level: u8,
    pub text: String,
    /// Slug used as the anchor, unique within the document.
    pub id: String,
}

/// Convert Markdown to HTML, discarding the outline.
pub(crate) fn to_html(markdown: &str) -> String {
    to_html_with_outline(markdown).0
}

/// Convert Markdown to HTML, highlighting fenced Soli blocks, and collect the
/// document's outline.
///
/// Headings get slug `id`s as a side effect — the outline is only useful if
/// its entries have something to link to.
pub(crate) fn to_html_with_outline(markdown: &str) -> (String, Vec<Heading>) {
    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TASKLISTS);

    let mut events: Vec<Event> = Vec::new();
    // `Some(lang)` while inside a fence; the text of the block accumulates in
    // `code` so it can be lexed as a whole rather than event by event.
    let mut fence: Option<String> = None;
    let mut code = String::new();

    let mut outline: Vec<Heading> = Vec::new();
    // Where the open heading's `Start` sits in `events`, and its level. The id
    // can only be written once the heading's text has been read, which arrives
    // as later events — so the `Start` is patched in place at `End`.
    let mut open_heading: Option<(usize, u8)> = None;
    let mut heading_text = String::new();
    let mut used_ids: Vec<String> = Vec::new();

    for event in Parser::new_ext(markdown, options) {
        match event {
            Event::Start(Tag::CodeBlock(kind)) => {
                fence = Some(match kind {
                    CodeBlockKind::Fenced(info) => {
                        info.split_whitespace().next().unwrap_or("").to_string()
                    }
                    CodeBlockKind::Indented => String::new(),
                });
                code.clear();
            }
            Event::End(TagEnd::CodeBlock) => {
                let lang = fence.take().unwrap_or_default();
                events.push(Event::Html(render_code(&lang, &code).into()));
                code.clear();
            }
            // Everything between the fence markers is literal code text.
            other if fence.is_some() => {
                if let Event::Text(text) = other {
                    code.push_str(&text);
                }
            }
            Event::Start(Tag::Heading {
                level,
                id,
                classes,
                attrs,
            }) => {
                open_heading = Some((events.len(), level as u8));
                heading_text.clear();
                events.push(Event::Start(Tag::Heading {
                    level,
                    id,
                    classes,
                    attrs,
                }));
            }
            Event::End(TagEnd::Heading(level)) => {
                if let Some((index, _)) = open_heading.take() {
                    let text = heading_text.trim().to_string();
                    let id = unique_slug(&text, &mut used_ids);
                    if let Event::Start(Tag::Heading { id: slot, .. }) = &mut events[index] {
                        *slot = Some(id.clone().into());
                    }
                    // `h1` is the page title, already shown in the masthead;
                    // anything below `h3` is too fine-grained to navigate by.
                    let level = level as u8;
                    if (2..=3).contains(&level) && !text.is_empty() {
                        outline.push(Heading { level, text, id });
                    }
                }
                events.push(Event::End(TagEnd::Heading(level)));
            }
            // Heading text arrives as ordinary inline events; mirror it into
            // the slug buffer on the way through.
            Event::Text(text) if open_heading.is_some() => {
                heading_text.push_str(&text);
                events.push(Event::Text(text));
            }
            Event::Code(code_text) if open_heading.is_some() => {
                heading_text.push_str(&code_text);
                events.push(Event::Code(code_text));
            }
            Event::Start(Tag::Link {
                link_type,
                dest_url,
                title,
                id,
            }) => events.push(Event::Start(Tag::Link {
                link_type,
                dest_url: safe_markdown_url(dest_url),
                title,
                id,
            })),
            Event::Start(Tag::Image {
                link_type,
                dest_url,
                title,
                id,
            }) => events.push(Event::Start(Tag::Image {
                link_type,
                dest_url: safe_markdown_url(dest_url),
                title,
                id,
            })),
            other => events.push(other),
        }
    }

    let mut out = String::new();
    pulldown_cmark::html::push_html(&mut out, events.into_iter());
    (out, outline)
}

/// Slugify a heading, disambiguating repeats with a numeric suffix so two
/// sections called "Options" still get their own anchor.
fn unique_slug(text: &str, used: &mut Vec<String>) -> String {
    let mut slug = String::with_capacity(text.len());
    let mut last_dash = true; // leading dashes are dropped
    for ch in text.chars() {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch.to_ascii_lowercase());
            last_dash = false;
        } else if !last_dash {
            slug.push('-');
            last_dash = true;
        }
    }
    let base = slug.trim_matches('-').to_string();
    let base = if base.is_empty() {
        "section".to_string()
    } else {
        base
    };

    let mut candidate = base.clone();
    let mut n = 2;
    while used.contains(&candidate) {
        candidate = format!("{}-{}", base, n);
        n += 1;
    }
    used.push(candidate.clone());
    candidate
}

/// The outline as a right-hand navigation rail. Empty when the document has
/// fewer than two headings — a table of contents with one line in it is
/// furniture, not navigation.
pub(crate) fn render_outline(outline: &[Heading]) -> String {
    if outline.len() < 2 {
        return String::new();
    }

    let mut out = String::with_capacity(outline.len() * 80);
    out.push_str(
        "<nav class=\"toc\" aria-label=\"On this page\"><p class=\"lbl\">On this page</p>",
    );
    for heading in outline {
        out.push_str(&format!(
            "<a class=\"{}\" href=\"#{}\">{}</a>",
            if heading.level == 3 { "sub" } else { "top" },
            html_escape(&heading.id),
            html_escape(&heading.text)
        ));
    }
    out.push_str("</nav>");
    out
}

/// Render one fenced block. The info string is kept as a `data-lang` label —
/// it is what the author wrote, not a guess.
fn render_code(lang: &str, code: &str) -> String {
    let body = if SOLI_LANGUAGES.contains(&lang.to_ascii_lowercase().as_str()) {
        code.lines()
            .map(html_highlight_soli)
            .collect::<Vec<_>>()
            .join("\n")
    } else {
        html_escape(code).into_owned()
    };

    if lang.is_empty() {
        format!("<pre class=\"code\"><code>{}</code></pre>", body)
    } else {
        format!(
            "<pre class=\"code\" data-lang=\"{}\"><code>{}</code></pre>",
            html_escape(lang),
            body
        )
    }
}

/// The document's first level-1 heading, used as the page title.
fn first_heading(markdown: &str) -> Option<String> {
    markdown
        .lines()
        .find(|line| line.starts_with("# "))
        .map(|line| line.trim_start_matches("# ").trim().to_string())
        .filter(|title| !title.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn highlights_soli_fences_only() {
        let html = to_html("```soli\nlet x = 1\n```\n\n```python\nx = 1\n```");
        assert!(html.contains("data-lang=\"soli\""));
        assert!(html.contains("tok-kw"));
        // The python block is present but untouched by the lexer.
        assert!(html.contains("data-lang=\"python\""));
        let python_block = html.split("data-lang=\"python\"").nth(1).unwrap();
        assert!(!python_block.contains("tok-kw"));
    }

    #[test]
    fn escapes_code_that_looks_like_markup() {
        let html = to_html("```\n<script>alert(1)</script>\n```");
        assert!(!html.contains("<script>"));
        assert!(html.contains("&lt;script&gt;"));
    }

    #[test]
    fn neutralizes_javascript_urls() {
        let html = to_html("[click](javascript:alert(1))");
        assert!(!html.contains("javascript:"));
        assert!(html.contains("href=\"#\""));
    }

    #[test]
    fn keeps_ordinary_links() {
        let html = to_html("[docs](https://soli.dev/docs)");
        assert!(html.contains("href=\"https://soli.dev/docs\""));
    }

    #[test]
    fn renders_tables_and_task_lists() {
        let html = to_html("| a | b |\n|---|---|\n| 1 | 2 |");
        assert!(html.contains("<table>"));
        let tasks = to_html("- [x] done\n- [ ] todo");
        assert!(tasks.contains("type=\"checkbox\""));
    }

    #[test]
    fn outline_collects_h2_and_h3_with_anchors() {
        let (html, outline) =
            to_html_with_outline("# Title\n\n## Setup\n\ntext\n\n### Details\n\n## Usage");
        let levels: Vec<u8> = outline.iter().map(|h| h.level).collect();
        let ids: Vec<&str> = outline.iter().map(|h| h.id.as_str()).collect();
        // h1 is the page title and stays out of the rail.
        assert_eq!(levels, vec![2, 3, 2]);
        assert_eq!(ids, vec!["setup", "details", "usage"]);
        // The anchors the rail links to exist in the document.
        assert!(html.contains("id=\"setup\""));
        assert!(html.contains("id=\"details\""));
    }

    #[test]
    fn repeated_headings_get_distinct_anchors() {
        let (_, outline) = to_html_with_outline("## Options\n\n## Options\n\n## Options");
        let ids: Vec<&str> = outline.iter().map(|h| h.id.as_str()).collect();
        assert_eq!(ids, vec!["options", "options-2", "options-3"]);
    }

    #[test]
    fn headings_containing_code_and_punctuation_slug_cleanly() {
        let (_, outline) = to_html_with_outline("## The `grouped()` helper — notes!");
        assert_eq!(outline[0].id, "the-grouped-helper-notes");
        assert_eq!(outline[0].text, "The grouped() helper — notes!");
    }

    #[test]
    fn a_heading_inside_a_fence_is_not_an_entry() {
        let (_, outline) = to_html_with_outline("## Real\n\n```\n## Not a heading\n```");
        assert_eq!(outline.len(), 1);
        assert_eq!(outline[0].text, "Real");
    }

    #[test]
    fn a_single_heading_gets_no_rail() {
        // One line is furniture, not navigation.
        let (_, one) = to_html_with_outline("## Only");
        assert!(render_outline(&one).is_empty());
        let (_, two) = to_html_with_outline("## One\n\n## Two");
        assert!(render_outline(&two).contains("On this page"));
    }

    #[test]
    fn outline_escapes_heading_text() {
        let (_, outline) = to_html_with_outline("## One\n\n## Fish & Chips");
        let rail = render_outline(&outline);
        assert!(rail.contains("Fish &amp; Chips"));
        assert_eq!(outline[1].id, "fish-chips");
    }

    #[test]
    fn inline_html_in_a_heading_never_reaches_the_rail_as_markup() {
        // pulldown-cmark passes a developer's inline HTML through to the
        // document (the same policy as the `.md` view pipeline), but the rail
        // is built from text events only — so no tag can ride into it.
        let (_, outline) = to_html_with_outline("## One\n\n## <script>alert(1)</script>");
        let rail = render_outline(&outline);
        assert!(!rail.contains("<script"));
        assert!(rail.contains("alert(1)"));
    }

    #[test]
    fn first_heading_becomes_the_title() {
        assert_eq!(
            first_heading("# Getting started\n\ntext"),
            Some("Getting started".to_string())
        );
        assert_eq!(first_heading("no heading here"), None);
        assert_eq!(first_heading("## only h2"), None);
    }
}
