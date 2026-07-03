//! Markdown → PDF: fold a Markdown document into the JSON layout template the
//! `soli-pdf` engine already renders. Headings, paragraphs, lists (nested),
//! tables, code blocks, blockquotes, rules and images map onto the existing
//! elements; inline `**bold**` / `*italic*` / `` `code` `` / `[link](url)` /
//! `~~strike~~` become styled `spans`. A [`Theme`] controls fonts, sizes and
//! colours.
//!
//! This keeps the "write prose, get a designed PDF" ergonomics a thin transform
//! over the engine rather than a second renderer.

use pulldown_cmark::{Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use serde_json::{json, Value as J};

/// Usable content width on A4 portrait after ~14 mm margins, in pt. Sizes table
/// columns evenly.
const CONTENT_WIDTH: f32 = 515.0;

/// Visual theme for a Markdown render. All fields default sensibly; the builtin
/// overrides them from the `options` hash.
#[derive(Debug, Clone)]
pub struct Theme {
    pub fonts: Vec<String>,
    pub body_size: f32,
    /// Font sizes for H1..H6.
    pub heading_sizes: [f32; 6],
    pub heading_color: Option<String>,
    pub text_color: Option<String>,
    pub link_color: String,
    pub code_color: String,
    pub line_height: f32,
    pub paragraph_spacing: f32,
}

impl Default for Theme {
    fn default() -> Self {
        Theme {
            fonts: vec!["titillium".to_string()],
            body_size: 11.0,
            heading_sizes: [23.0, 18.0, 15.0, 13.0, 12.0, 11.0],
            heading_color: Some("111827".to_string()),
            text_color: None,
            link_color: "0f766e".to_string(),
            code_color: "9a3412".to_string(),
            line_height: 1.45,
            paragraph_spacing: 8.0,
        }
    }
}

/// One inline styled run.
#[derive(Default, Clone)]
struct Span {
    text: String,
    bold: bool,
    italic: bool,
    code: bool,
    strike: bool,
    link: Option<String>,
}

impl Span {
    fn style_eq(&self, o: &Span) -> bool {
        self.bold == o.bold
            && self.italic == o.italic
            && self.code == o.code
            && self.strike == o.strike
            && self.link == o.link
    }
    fn to_json(&self, theme: &Theme) -> J {
        let mut o = json!({ "text": self.text });
        let m = o.as_object_mut().unwrap();
        if self.bold {
            m.insert("fontWeight".into(), json!("bold"));
        }
        if self.italic {
            m.insert("italic".into(), json!(true));
        }
        if self.strike {
            m.insert("strike".into(), json!(true));
        }
        if self.code {
            m.insert("mono".into(), json!(true));
            m.insert("color".into(), json!(theme.code_color));
        }
        if let Some(url) = &self.link {
            m.insert("link".into(), json!(url));
            m.insert("color".into(), json!(theme.link_color));
            m.insert("underline".into(), json!(true));
        }
        o
    }
}

/// A list being assembled. `item_spans`/`item_child` accumulate the item that is
/// currently open; on End(Item) they fold into `items`.
struct ListFrame {
    ordered: bool,
    start: i64,
    items: Vec<J>,
    item_spans: Vec<Span>,
    item_child: Option<J>,
}

/// Streaming folder from Markdown events to template elements.
struct Builder<'t> {
    theme: &'t Theme,
    content: Vec<J>,
    inline: Vec<Span>, // top-level paragraph/heading buffer
    heading: Option<u8>,
    quote_depth: u32,
    // inline style state
    bold: u32,
    italic: u32,
    strike: u32,
    code: bool,
    link: Option<String>,
    // lists (nesting stack)
    lists: Vec<ListFrame>,
    // code block
    in_code: bool,
    code_buf: String,
    // image capture (swallow alt text)
    in_image: u32,
    // table
    table_open: bool,
    in_table_head: bool,
    table_headers: Vec<String>,
    table_rows: Vec<Vec<String>>,
    cur_row: Vec<String>,
    cell: String,
}

fn heading_num(l: HeadingLevel) -> u8 {
    match l {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}

impl<'t> Builder<'t> {
    fn new(theme: &'t Theme) -> Self {
        Builder {
            theme,
            content: Vec::new(),
            inline: Vec::new(),
            heading: None,
            quote_depth: 0,
            bold: 0,
            italic: 0,
            strike: 0,
            code: false,
            link: None,
            lists: Vec::new(),
            in_code: false,
            code_buf: String::new(),
            in_image: 0,
            table_open: false,
            in_table_head: false,
            table_headers: Vec::new(),
            table_rows: Vec::new(),
            cur_row: Vec::new(),
            cell: String::new(),
        }
    }

    fn push_text(&mut self, text: &str) {
        if self.in_image > 0 {
            return; // alt text is captured via the image tag, not rendered
        }
        if self.in_code {
            self.code_buf.push_str(text);
            return;
        }
        if self.table_open {
            self.cell.push_str(text);
            return;
        }
        let target = if let Some(frame) = self.lists.last_mut() {
            &mut frame.item_spans
        } else {
            &mut self.inline
        };
        let want = Span {
            bold: self.bold > 0,
            italic: self.italic > 0,
            strike: self.strike > 0,
            code: self.code,
            link: self.link.clone(),
            text: String::new(),
        };
        match target.last_mut() {
            Some(last) if last.style_eq(&want) => last.text.push_str(text),
            _ => {
                let mut s = want;
                s.text.push_str(text);
                target.push(s);
            }
        }
    }

    fn spans_json(&self, spans: &[Span]) -> Vec<J> {
        spans.iter().map(|s| s.to_json(self.theme)).collect()
    }

    fn flush_paragraph(&mut self) {
        if self.inline.iter().all(|s| s.text.trim().is_empty()) {
            self.inline.clear();
            return;
        }
        let spans = self.spans_json(&self.inline);
        let el = if let Some(level) = self.heading {
            let idx = (level.clamp(1, 6) - 1) as usize;
            let mut opts = json!({
                "fontSize": self.theme.heading_sizes[idx],
                "fontWeight": "bold",
                "spacing": self.theme.paragraph_spacing + (6.0 - idx as f32).max(0.0),
                "minSpaceBelow": 30.0,
            });
            if let Some(c) = &self.theme.heading_color {
                opts.as_object_mut()
                    .unwrap()
                    .insert("color".into(), json!(c));
            }
            json!({ "type": "paragraph", "spans": spans, "options": opts })
        } else if self.quote_depth > 0 {
            json!({ "type": "paragraph", "spans": spans, "options": {
                "fontSize": self.theme.body_size, "italic": true, "color": "6b7280",
                "lineHeight": self.theme.line_height, "spacing": self.theme.paragraph_spacing,
            }})
        } else {
            let mut opts = json!({
                "fontSize": self.theme.body_size,
                "lineHeight": self.theme.line_height,
                "spacing": self.theme.paragraph_spacing,
            });
            if let Some(c) = &self.theme.text_color {
                opts.as_object_mut()
                    .unwrap()
                    .insert("color".into(), json!(c));
            }
            json!({ "type": "paragraph", "spans": spans, "options": opts })
        };
        self.inline.clear();
        self.content.push(el);
    }

    fn build_list_json(&self, frame: &ListFrame, depth: usize) -> J {
        json!({
            "type": "list",
            "ordered": frame.ordered,
            "start": frame.start,
            "indent": 18.0,
            "spacing": 3.0,
            "items": frame.items,
            "options": { "fontSize": self.theme.body_size, "lineHeight": self.theme.line_height },
            "_depth": depth, // ignored by the engine; documents nesting intent
        })
    }

    fn finish_item(&mut self) {
        let Some(mut frame) = self.lists.pop() else {
            return;
        };
        let spans = self.spans_json(&frame.item_spans);
        frame.item_spans.clear();
        let item = match frame.item_child.take() {
            Some(child) => {
                // child is a full `list` element; convert to the item's nested list.
                let mut node = json!({ "spans": spans });
                if let Some(list_obj) = child.as_object() {
                    // Reuse the child list's structural keys as a nested `list`.
                    let nested: J = json!({
                        "ordered": list_obj.get("ordered").cloned().unwrap_or(json!(false)),
                        "start": list_obj.get("start").cloned().unwrap_or(json!(1)),
                        "indent": 18.0,
                        "spacing": 3.0,
                        "items": list_obj.get("items").cloned().unwrap_or(json!([])),
                        "options": list_obj.get("options").cloned().unwrap_or(json!({})),
                    });
                    node.as_object_mut().unwrap().insert("list".into(), nested);
                }
                node
            }
            None => json!({ "spans": spans }),
        };
        frame.items.push(item);
        self.lists.push(frame);
    }

    fn emit_table(&mut self) {
        let ncols = self.table_headers.len().max(1);
        let width = CONTENT_WIDTH / ncols as f32;
        let header_columns: Vec<J> = self
            .table_headers
            .iter()
            .map(|h| {
                json!({ "text": h.trim(), "width": width, "fontWeight": "bold",
                             "borderSides": { "bottom": true } })
            })
            .collect();
        let rows: Vec<J> = self
            .table_rows
            .iter()
            .map(|row| {
                J::Array(
                    (0..ncols)
                        .map(|i| {
                            json!({ "text": row.get(i).map(|s| s.trim()).unwrap_or(""),
                                         "width": width })
                        })
                        .collect(),
                )
            })
            .collect();
        self.content.push(json!({
            "type": "table",
            "header_columns": header_columns,
            "rows": rows,
            "options": { "padding_x": 6, "padding_y": 5, "stripe": "f5f5f4",
                         "header": { "textColor": "0f172a" } }
        }));
        self.content
            .push(json!({ "type": "move", "y": self.theme.paragraph_spacing }));
        self.table_headers.clear();
        self.table_rows.clear();
    }

    fn emit_code_block(&mut self) {
        let code = std::mem::take(&mut self.code_buf);
        let lines: Vec<&str> = code.trim_end_matches('\n').split('\n').collect();
        self.content.push(json!({ "type": "move", "y": 2.0 }));
        for line in lines {
            self.content.push(json!({
                "type": "paragraph",
                "spans": [{ "text": if line.is_empty() { " " } else { line }, "mono": true, "color": "334155" }],
                "options": { "fontSize": self.theme.body_size - 1.0, "lineHeight": 1.2, "color": "334155" }
            }));
        }
        self.content
            .push(json!({ "type": "move", "y": self.theme.paragraph_spacing }));
    }

    fn event(&mut self, ev: Event) {
        match ev {
            Event::Start(tag) => self.start(tag),
            Event::End(tag) => self.end(tag),
            Event::Text(t) => self.push_text(&t),
            Event::Code(t) => {
                self.code = true;
                self.push_text(&t);
                self.code = false;
            }
            Event::SoftBreak => self.push_text(" "),
            Event::HardBreak => self.push_text("\n"),
            Event::Rule => {
                self.flush_paragraph();
                self.content.push(json!({ "type": "hr" }));
                self.content
                    .push(json!({ "type": "move", "y": self.theme.paragraph_spacing }));
            }
            _ => {}
        }
    }

    fn start(&mut self, tag: Tag) {
        match tag {
            Tag::Paragraph => {}
            Tag::Heading { level, .. } => self.heading = Some(heading_num(level)),
            Tag::BlockQuote(_) => self.quote_depth += 1,
            Tag::CodeBlock(_) => {
                self.in_code = true;
                self.code_buf.clear();
            }
            Tag::List(start) => {
                self.lists.push(ListFrame {
                    ordered: start.is_some(),
                    start: start.map(|s| s as i64).unwrap_or(1),
                    items: Vec::new(),
                    item_spans: Vec::new(),
                    item_child: None,
                });
            }
            Tag::Item => {
                if let Some(frame) = self.lists.last_mut() {
                    frame.item_spans.clear();
                    frame.item_child = None;
                }
            }
            Tag::Table(_) => {
                self.table_open = true;
                self.table_headers.clear();
                self.table_rows.clear();
            }
            Tag::TableHead => self.in_table_head = true,
            Tag::TableRow => self.cur_row.clear(),
            Tag::TableCell => self.cell.clear(),
            Tag::Emphasis => self.italic += 1,
            Tag::Strong => self.bold += 1,
            Tag::Strikethrough => self.strike += 1,
            Tag::Link { dest_url, .. } => self.link = Some(dest_url.to_string()),
            Tag::Image { dest_url, .. } => {
                self.flush_paragraph();
                self.content
                    .push(json!({ "type": "image", "value": dest_url.to_string() }));
                self.in_image += 1;
            }
            _ => {}
        }
    }

    fn end(&mut self, tag: TagEnd) {
        match tag {
            // A paragraph inside a list item is transparent — its text already
            // routed to the item; only flush free-standing paragraphs.
            TagEnd::Paragraph if self.lists.is_empty() => self.flush_paragraph(),
            TagEnd::Heading(_) => {
                self.flush_paragraph();
                self.heading = None;
            }
            TagEnd::BlockQuote(_) => self.quote_depth = self.quote_depth.saturating_sub(1),
            TagEnd::CodeBlock => {
                self.in_code = false;
                self.emit_code_block();
            }
            TagEnd::List(_) => {
                if let Some(frame) = self.lists.pop() {
                    let depth = self.lists.len();
                    let list_json = self.build_list_json(&frame, depth);
                    if let Some(parent) = self.lists.last_mut() {
                        // Nested list: attach to the parent's open item.
                        parent.item_child = Some(list_json);
                    } else {
                        self.content.push(list_json);
                        self.content.push(
                            json!({ "type": "move", "y": self.theme.paragraph_spacing / 2.0 }),
                        );
                    }
                }
            }
            TagEnd::Item => self.finish_item(),
            TagEnd::Table => {
                self.emit_table();
                self.table_open = false;
            }
            TagEnd::TableHead => self.in_table_head = false,
            TagEnd::TableRow if !self.in_table_head => {
                self.table_rows.push(std::mem::take(&mut self.cur_row));
            }
            TagEnd::TableCell => {
                let cell = std::mem::take(&mut self.cell);
                if self.in_table_head {
                    self.table_headers.push(cell);
                } else {
                    self.cur_row.push(cell);
                }
            }
            TagEnd::Emphasis => self.italic = self.italic.saturating_sub(1),
            TagEnd::Strong => self.bold = self.bold.saturating_sub(1),
            TagEnd::Strikethrough => self.strike = self.strike.saturating_sub(1),
            TagEnd::Link => self.link = None,
            TagEnd::Image => self.in_image = self.in_image.saturating_sub(1),
            _ => {}
        }
    }
}

/// Convert a Markdown document to a `soli-pdf` template JSON value.
pub fn markdown_to_template(md: &str, theme: &Theme) -> J {
    let mut opts = Options::empty();
    opts.insert(Options::ENABLE_TABLES);
    opts.insert(Options::ENABLE_STRIKETHROUGH);
    opts.insert(Options::ENABLE_TASKLISTS);
    opts.insert(Options::ENABLE_FOOTNOTES);

    let mut b = Builder::new(theme);
    for event in Parser::new_ext(md, opts) {
        b.event(event);
    }
    b.flush_paragraph();

    json!({
        "fonts": theme.fonts,
        "options": { "margins": 40 },
        "content": b.content,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn render(md: &str) -> J {
        markdown_to_template(md, &Theme::default())
    }

    fn content(t: &J) -> Vec<J> {
        t.get("content").unwrap().as_array().unwrap().clone()
    }

    #[test]
    fn heading_and_paragraph() {
        let t = render("# Title\n\nHello **world**.");
        let c = content(&t);
        assert_eq!(c[0]["type"], "paragraph");
        assert_eq!(c[0]["options"]["fontWeight"], "bold");
        assert_eq!(c[0]["spans"][0]["text"], "Title");
        // The bold run is its own span.
        let spans = c[1]["spans"].as_array().unwrap();
        assert!(spans
            .iter()
            .any(|s| s["text"] == "world" && s["fontWeight"] == "bold"));
    }

    #[test]
    fn bullet_list_items() {
        let t = render("- one\n- two");
        let c = content(&t);
        let list = c.iter().find(|e| e["type"] == "list").unwrap();
        assert_eq!(list["ordered"], false);
        assert_eq!(list["items"].as_array().unwrap().len(), 2);
        assert_eq!(list["items"][0]["spans"][0]["text"], "one");
    }

    #[test]
    fn ordered_list_start() {
        let t = render("3. c\n4. d");
        let list = content(&t)
            .into_iter()
            .find(|e| e["type"] == "list")
            .unwrap();
        assert_eq!(list["ordered"], true);
        assert_eq!(list["start"], 3);
    }

    #[test]
    fn nested_list_becomes_a_sublist() {
        let t = render("- parent\n    - child");
        let list = content(&t)
            .into_iter()
            .find(|e| e["type"] == "list")
            .unwrap();
        let parent_item = &list["items"][0];
        assert_eq!(parent_item["spans"][0]["text"], "parent");
        assert_eq!(parent_item["list"]["items"][0]["spans"][0]["text"], "child");
    }

    #[test]
    fn table_headers_and_rows() {
        let t = render("| A | B |\n|---|---|\n| 1 | 2 |");
        let table = content(&t)
            .into_iter()
            .find(|e| e["type"] == "table")
            .unwrap();
        assert_eq!(table["header_columns"][0]["text"], "A");
        assert_eq!(table["rows"][0][1]["text"], "2");
    }

    #[test]
    fn code_block_is_mono() {
        let t = render("```\nlet x = 1\n```");
        let mono = content(&t)
            .into_iter()
            .find(|e| e["type"] == "paragraph" && e["spans"][0]["mono"] == true);
        assert_eq!(mono.unwrap()["spans"][0]["text"], "let x = 1");
    }

    #[test]
    fn link_span_carries_url() {
        let t = render("[click](https://example.com)");
        let c = content(&t);
        let span = &c[0]["spans"][0];
        assert_eq!(span["link"], "https://example.com");
        assert_eq!(span["text"], "click");
    }
}
