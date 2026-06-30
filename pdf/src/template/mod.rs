//! Serde model for the JSON layout template.
//!
//! Field names mostly follow the camelCase used in the template
//! (`fontSize`, `fontWeight`, `borderSides`); a few top-level keys are
//! snake_case (`header_columns`, `header_height`, `padding_x`) and are renamed
//! per field.

mod cell;
pub use cell::*;

use serde::Deserialize;

use crate::error::{PdfError, Result};

/// Horizontal text alignment. Parsed case-insensitively (`"Left"` == `"left"`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Alignment {
    #[default]
    Left,
    Right,
    Center,
}

/// Font weight. Parsed case-insensitively.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum FontWeight {
    #[default]
    Normal,
    Bold,
}

/// The whole template document.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Template {
    #[serde(default)]
    pub fonts: Vec<String>,
    #[serde(default)]
    pub options: TemplateOptions,
    #[serde(default)]
    pub header: Vec<Element>,
    #[serde(default)]
    pub footer: Vec<Element>,
    #[serde(default)]
    pub content: Vec<Element>,
}

impl Template {
    /// Parse a template from JSON bytes.
    pub fn parse(bytes: &[u8]) -> Result<Template> {
        serde_json::from_slice(bytes).map_err(PdfError::from)
    }
}

/// Document-level options.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct TemplateOptions {
    /// Height (pt) of the header band reserved at the top of every page.
    #[serde(default)]
    pub header_height: f32,
    /// Optional diagonal watermark/stamp drawn behind the content of every page.
    #[serde(default)]
    pub watermark: Option<Watermark>,
    /// Page margins (pt). A single number applies to all four sides; an object
    /// overrides individual sides. Unset sides default to ~20 mm. The top margin
    /// is the gap above the header; the bottom margin the gap below the footer.
    #[serde(default)]
    pub margins: Option<MarginsSpec>,
    /// Page size: a preset name (`a4`/`letter`/`legal`/`a5`/`a3`) or a custom
    /// `{ width, height }` in points. Defaults to A4.
    #[serde(default)]
    pub page: Option<PageSpec>,
    /// `"portrait"` (default) or `"landscape"` (swaps width/height).
    #[serde(default)]
    pub orientation: Option<String>,
    /// Page background fill color (hex, no `#`) painted behind every page,
    /// beneath any watermark and content. Defaults to none (white paper).
    #[serde(default)]
    pub background: Option<String>,
}

/// Page size: a preset name or explicit dimensions in points.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum PageSpec {
    Named(String),
    Custom { width: f32, height: f32 },
}

/// Page margins: either one value for all sides, or per-side overrides.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum MarginsSpec {
    All(f32),
    Sides(MarginSides),
}

/// Per-side margin overrides (pt); unset sides fall back to the default.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct MarginSides {
    #[serde(default)]
    pub top: Option<f32>,
    #[serde(default)]
    pub right: Option<f32>,
    #[serde(default)]
    pub bottom: Option<f32>,
    #[serde(default)]
    pub left: Option<f32>,
}

impl MarginsSpec {
    /// Resolve to `(top, right, bottom, left)` in points, filling unset sides
    /// with `default` and clamping negatives to zero.
    pub fn resolve(&self, default: f32) -> (f32, f32, f32, f32) {
        match self {
            MarginsSpec::All(v) => {
                let v = v.max(0.0);
                (v, v, v, v)
            }
            MarginsSpec::Sides(s) => (
                s.top.unwrap_or(default).max(0.0),
                s.right.unwrap_or(default).max(0.0),
                s.bottom.unwrap_or(default).max(0.0),
                s.left.unwrap_or(default).max(0.0),
            ),
        }
    }
}

/// A diagonal page watermark/stamp (e.g. "DRAFT", "PAID", "COPY"), centered and
/// drawn behind the page content.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Watermark {
    pub text: String,
    /// Rotation in degrees (default 45).
    #[serde(default = "default_watermark_angle")]
    pub angle: f32,
    /// Fill color (hex, no `#`); defaults to a light grey.
    #[serde(default)]
    pub color: Option<String>,
    #[serde(default = "default_watermark_size")]
    pub font_size: f32,
    #[serde(default = "default_bold_weight", deserialize_with = "de_font_weight")]
    pub font_weight: FontWeight,
    /// Draw the stamp *on top of* the content (an overlay) instead of behind it.
    /// Default `false` (behind). Use `true` so panels/images can't hide it.
    #[serde(default)]
    pub front: bool,
    /// Explicit center X (pt) of the stamp. Defaults to the page horizontal center.
    #[serde(default)]
    pub x: Option<f32>,
    /// Explicit center Y (pt) of the stamp. Defaults to the page vertical center.
    #[serde(default)]
    pub y: Option<f32>,
    /// Convenience vertical placement when `y` is unset: `"top"` | `"center"` |
    /// `"bottom"`. Ignored if `y` is given.
    #[serde(default)]
    pub anchor: Option<String>,
    /// Which pages to stamp: `"all"` (default) / `"first"` / `"last"`, or an
    /// explicit list of 1-based page numbers (e.g. `[1, 3]`).
    #[serde(default = "default_page_filter")]
    pub pages: PageFilter,
}

/// Which pages a document watermark applies to.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum PageFilter {
    /// `"all"` | `"first"` | `"last"`.
    Keyword(String),
    /// Explicit 1-based page numbers.
    Pages(Vec<usize>),
}

impl PageFilter {
    /// Whether page `pageno` (1-based) of a `total`-page document is stamped.
    pub fn matches(&self, pageno: usize, total: usize) -> bool {
        match self {
            PageFilter::Keyword(k) => match k.to_ascii_lowercase().as_str() {
                "first" => pageno == 1,
                "last" => pageno == total,
                _ => true, // "all" or anything unrecognized
            },
            PageFilter::Pages(ps) => ps.contains(&pageno),
        }
    }
}

/// One flow element.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Element {
    Paragraph(Paragraph),
    Move(MoveOp),
    Image(ImageEl),
    Table(Table),
    /// A horizontal rule (full content width, or `width` pt) drawn at the
    /// cursor; advances the cursor below it.
    Hr(HrEl),
    /// A filled and/or stroked rectangle placed at the cursor (top-left). Does
    /// not advance the cursor — position it with `move`.
    Rect(RectEl),
    /// A stroked line segment from the cursor to `cursor + (dx, dy)`. Does not
    /// advance the cursor.
    Line(LineEl),
    /// A QR code (EPC/GiroCode "scan-to-pay" or arbitrary text) rendered as a
    /// raster image at the cursor. Does not advance the cursor.
    Qr(QrEl),
    /// A 1D barcode (Code 128 / EAN-13 / EAN-8 / Code 39) rasterised at the
    /// cursor. Does not advance the cursor.
    Barcode(BarcodeEl),
    /// A filled and/or stroked ellipse/circle at the cursor (top-left of its
    /// bounding box). Does not advance the cursor.
    Ellipse(EllipseEl),
    /// A bulleted or numbered list (items may be plain text, inline rich text,
    /// or carry a nested sublist). Flows like a paragraph and advances the cursor.
    List(ListEl),
    /// A bar, line, or pie chart drawn from data-bound or inline points. Flows
    /// like a block and advances the cursor below it.
    Chart(ChartEl),
    /// Render `content` once per item of a data array, with `${field}` scoped to
    /// each item. The block-level analogue of a data-bound table.
    Repeat(RepeatEl),
    /// Render `content` only when the `when` condition holds (else `else`).
    If(Conditional),
    /// The negation of `if`: render `content` only when the condition is false.
    Unless(Conditional),
}

/// A `repeat` block: `content` is laid out once per item of the `data` array,
/// with `${field}` resolving against each item (falling back to the root), just
/// like a data-bound table row. Nesting is supported, but a `table`/`chart`
/// inside a `repeat` still binds its own `data` key against the top-level data,
/// not the current item.
#[derive(Debug, Clone, Deserialize)]
pub struct RepeatEl {
    /// Key of an array in the data document; `content` repeats once per element.
    pub data: String,
    /// The elements rendered per item.
    #[serde(default)]
    pub content: Vec<Element>,
}

/// An `if` / `unless` block. The test reads `${when}`: with `equals`, it's a
/// string-equality check; otherwise a truthiness check (a value is falsy when it
/// is missing, empty, `false`, `0`, or `null`). `unless` negates the result.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Conditional {
    /// Dotted data path whose value drives the condition.
    pub when: String,
    /// When set, the test is `${when} == equals` instead of truthiness.
    #[serde(default)]
    pub equals: Option<String>,
    /// Elements rendered when the condition is satisfied.
    #[serde(default)]
    pub content: Vec<Element>,
    /// Elements rendered otherwise (optional).
    #[serde(default, rename = "else")]
    pub else_content: Vec<Element>,
}

/// A chart element. `kind` is `"bar"`, `"line"`, or `"pie"`. Values come either
/// from a data binding (`data` names an array; `label`/`value` name the fields
/// to read from each item) or from inline `points`.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChartEl {
    #[serde(default = "default_chart_kind")]
    pub kind: String,
    /// Chart box width (pt). Defaults to the full content width.
    #[serde(default)]
    pub width: f32,
    /// Chart box height (pt) for the plot area (title/labels add to the total).
    #[serde(default = "default_chart_height")]
    pub height: f32,
    /// Data-binding key: an array in the data document, one entry per point.
    #[serde(default)]
    pub data: Option<String>,
    /// Field name read for each point's label (default `"label"`).
    #[serde(default)]
    pub label: Option<String>,
    /// Field name read for each point's value (default `"value"`).
    #[serde(default)]
    pub value: Option<String>,
    /// Inline points, used when `data` is absent.
    #[serde(default)]
    pub points: Option<Vec<ChartPoint>>,
    /// Slice/bar/line colors (hex, no `#`), cycled across points. A built-in
    /// palette is used when empty.
    #[serde(default)]
    pub colors: Vec<String>,
    /// Draw a legend (pie only; bar/line label the axis instead). Default true.
    #[serde(default = "default_true")]
    pub legend: bool,
    /// Draw axis lines and category labels (bar/line). Default true.
    #[serde(default = "default_true")]
    pub axis: bool,
    /// Optional title drawn above the chart.
    #[serde(default)]
    pub title: Option<String>,
}

/// One inline chart point.
#[derive(Debug, Clone, Deserialize)]
pub struct ChartPoint {
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub value: f64,
}

/// A bulleted (unordered) or numbered (ordered) list.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListEl {
    /// Numbered (`1.`, `2.`, …) when true, bulleted when false (the default).
    #[serde(default)]
    pub ordered: bool,
    /// First number for an ordered list (default 1).
    #[serde(default = "default_list_start")]
    pub start: i64,
    /// Bullet glyph for an unordered list (default `"•"`). Must be a glyph the
    /// loaded font covers.
    #[serde(default)]
    pub marker: Option<String>,
    /// Left indent (pt) for this level, added on top of any parent level.
    #[serde(default = "default_list_indent")]
    pub indent: f32,
    /// Extra vertical gap (pt) after each item.
    #[serde(default)]
    pub spacing: f32,
    /// The list items.
    #[serde(default)]
    pub items: Vec<ListItem>,
    /// Base text styling (font size/weight/color) for item bodies.
    #[serde(default)]
    pub options: TextOptions,
}

/// One list item: a bare string, or a node with an explicit body and/or a
/// nested sublist.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum ListItem {
    /// `"plain text"` — the common case.
    Text(String),
    /// `{ "text"|"spans": …, "list": { … } }` — a richer item.
    Node(ListItemNode),
}

/// A list item with an explicit body and/or a nested sublist beneath it.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListItemNode {
    /// Plain-text body (`${...}`-interpolated).
    #[serde(default)]
    pub text: Option<String>,
    /// Inline rich-text body (takes precedence over `text`).
    #[serde(default)]
    pub spans: Option<Vec<StyledSpan>>,
    /// A nested sublist rendered below this item, indented one level deeper.
    #[serde(default)]
    pub list: Option<Box<ListEl>>,
}

/// A wrapped, aligned block of text. Either a plain `value` (single style) or
/// an array of styled `spans` (inline rich text). When `spans` is present it
/// takes precedence over `value`.
#[derive(Debug, Clone, Deserialize)]
pub struct Paragraph {
    #[serde(default)]
    pub value: String,
    /// Inline rich text: a sequence of styled segments rendered on one flow,
    /// wrapped together. Each span overrides the paragraph `options` it omits.
    #[serde(default)]
    pub spans: Option<Vec<StyledSpan>>,
    #[serde(default)]
    pub options: TextOptions,
}

/// One styled segment of inline rich text. Unset fields inherit the paragraph's
/// `options`.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StyledSpan {
    pub text: String,
    #[serde(default)]
    pub font_size: Option<f32>,
    #[serde(default, deserialize_with = "de_opt_font_weight")]
    pub font_weight: Option<FontWeight>,
    /// Italic face (requires an italic font in the family). Inherits if unset.
    #[serde(default)]
    pub italic: Option<bool>,
    /// Monospace face (uses the loaded mono font, e.g. for `code`). Inherits if unset.
    #[serde(default)]
    pub mono: Option<bool>,
    /// Text color (hex, no `#`); inherits black / the paragraph default if unset.
    #[serde(default)]
    pub color: Option<String>,
    /// External URL; makes this span a clickable link.
    #[serde(default)]
    pub link: Option<String>,
}

/// Text styling for paragraphs.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextOptions {
    #[serde(default, deserialize_with = "de_alignment")]
    pub alignment: Alignment,
    #[serde(default = "default_font_size")]
    pub font_size: f32,
    #[serde(default, deserialize_with = "de_font_weight")]
    pub font_weight: FontWeight,
    /// Render in the italic face (requires an italic font in the family).
    #[serde(default)]
    pub italic: bool,
    /// Render in the monospace face (the loaded mono font).
    #[serde(default)]
    pub mono: bool,
    /// Optional external URL; when set, the text becomes a clickable link.
    #[serde(default)]
    pub link: Option<String>,
    /// Add this paragraph to the PDF outline (sidebar bookmarks) with this label.
    #[serde(default)]
    pub bookmark: Option<String>,
    /// Name this paragraph as a jump target for `link_to`.
    #[serde(default)]
    pub anchor: Option<String>,
    /// Make the text a clickable internal jump to the paragraph with this `anchor`.
    /// Accepts `linkTo` (camelCase, consistent with `fontSize`) or `link_to`.
    #[serde(default, alias = "link_to")]
    pub link_to: Option<String>,
    /// Text fill color (hex, no `#`) for the whole paragraph (also list items and
    /// footer lines). Defaults to black. Use `spans` for per-run colors.
    #[serde(default)]
    pub color: Option<String>,
}

impl Default for TextOptions {
    fn default() -> Self {
        TextOptions {
            alignment: Alignment::Left,
            font_size: default_font_size(),
            font_weight: FontWeight::Normal,
            italic: false,
            mono: false,
            link: None,
            bookmark: None,
            anchor: None,
            link_to: None,
            color: None,
        }
    }
}

/// A relative cursor move. Positive `y` is DOWN the page, negative `y` is UP
/// (screen convention); positive `x` is right.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct MoveOp {
    #[serde(default)]
    pub x: f32,
    #[serde(default)]
    pub y: f32,
}

/// An image element (and inline image inside a rich cell).
#[derive(Debug, Clone, Deserialize)]
pub struct ImageEl {
    /// http(s) URL, `file://` path, or `data:` URI.
    pub value: String,
    /// Target width in pt; height is derived to preserve aspect ratio.
    #[serde(default)]
    pub width: f32,
}

/// A horizontal rule. Spans the content width unless `width` is given.
#[derive(Debug, Clone, Deserialize)]
pub struct HrEl {
    #[serde(default)]
    pub color: Option<String>,
    #[serde(default = "default_stroke_width")]
    pub thickness: f32,
    #[serde(default)]
    pub width: Option<f32>,
    /// Dash pattern (pt on/off lengths, e.g. `[3, 2]`); omit for a solid rule.
    #[serde(default)]
    pub dash: Option<Vec<f32>>,
}

/// A filled and/or stroked rectangle.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RectEl {
    #[serde(default)]
    pub width: f32,
    #[serde(default)]
    pub height: f32,
    /// Fill color (hex, no `#`); omit for no fill.
    #[serde(default)]
    pub fill: Option<String>,
    /// Border color (hex, no `#`); omit for no border.
    #[serde(default)]
    pub border: Option<String>,
    #[serde(default = "default_stroke_width")]
    pub border_width: f32,
    /// Corner radius (pt); when set, the rectangle is drawn with rounded corners.
    #[serde(default)]
    pub radius: Option<f32>,
    /// Border dash pattern (pt on/off lengths); omit for a solid border.
    #[serde(default)]
    pub dash: Option<Vec<f32>>,
}

/// A stroked line segment, relative to the cursor.
#[derive(Debug, Clone, Deserialize)]
pub struct LineEl {
    #[serde(default)]
    pub dx: f32,
    #[serde(default)]
    pub dy: f32,
    #[serde(default)]
    pub color: Option<String>,
    #[serde(default = "default_stroke_width")]
    pub width: f32,
    /// Dash pattern (pt on/off lengths); omit for a solid line.
    #[serde(default)]
    pub dash: Option<Vec<f32>>,
}

/// A filled and/or stroked ellipse. `rx`/`ry` are the radii (pt); a circle has
/// `rx == ry`.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EllipseEl {
    #[serde(default)]
    pub rx: f32,
    #[serde(default)]
    pub ry: f32,
    #[serde(default)]
    pub fill: Option<String>,
    #[serde(default)]
    pub border: Option<String>,
    #[serde(default = "default_stroke_width")]
    pub border_width: f32,
    #[serde(default)]
    pub dash: Option<Vec<f32>>,
}

/// A QR code element. `kind = "epc"` builds an EPC069-12 SEPA Credit Transfer
/// ("GiroCode") payload from the payment fields; `kind = "text"` encodes `value`
/// verbatim. All string fields are `${...}`-interpolated before encoding.
#[derive(Debug, Clone, Deserialize)]
pub struct QrEl {
    #[serde(default = "default_qr_kind")]
    pub kind: String,
    /// Raw payload for `kind = "text"`.
    #[serde(default)]
    pub value: Option<String>,
    // EPC (GiroCode) fields:
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub iban: Option<String>,
    #[serde(default)]
    pub bic: Option<String>,
    #[serde(default)]
    pub amount: Option<String>,
    #[serde(default)]
    pub currency: Option<String>,
    #[serde(default)]
    pub remittance: Option<String>,
    #[serde(default)]
    pub purpose: Option<String>,
    /// Rendered side length in pt (the QR is square).
    #[serde(default = "default_qr_width")]
    pub width: f32,
}

/// A 1D barcode element. `symbology` selects the encoding; `value` is the data
/// to encode (`${...}`-interpolated first). `code128` accepts any printable
/// ASCII; `ean13`/`ean8` need 12/7 digits (the check digit is computed);
/// `code39` accepts uppercase letters, digits and a few symbols.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BarcodeEl {
    #[serde(default = "default_barcode_symbology")]
    pub symbology: String,
    #[serde(default)]
    pub value: Option<String>,
    /// Rendered width in pt. The bar pattern is scaled to fit this width.
    #[serde(default = "default_barcode_width")]
    pub width: f32,
    /// Rendered bar height in pt.
    #[serde(default = "default_barcode_height")]
    pub height: f32,
    /// Print the encoded value as a caption centered below the bars.
    #[serde(default)]
    pub human_readable: bool,
}

/// A table element.
#[derive(Debug, Clone, Deserialize)]
pub struct Table {
    /// If set, the (single) template row in `rows` is repeated once per item of
    /// `data[<this key>]`, with `${field}` resolving against each item.
    #[serde(default)]
    pub data: Option<String>,
    #[serde(default)]
    pub header_columns: Vec<Cell>,
    #[serde(default)]
    pub rows: Vec<Vec<Cell>>,
    #[serde(default)]
    pub options: TableOptions,
    /// Optional stamp drawn centered over this table's box (always on top), e.g.
    /// a per-table `PAID`/`VOID` mark. `front`/`pages` on it are ignored here —
    /// it follows the table.
    #[serde(default)]
    pub watermark: Option<Watermark>,
}

/// Table-level options. Note these keys are snake_case in the template
/// (`padding_x`, `padding_y`), so no blanket camelCase rename here.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct TableOptions {
    #[serde(default)]
    pub header: TableHeaderStyle,
    #[serde(default)]
    pub padding_x: f32,
    #[serde(default)]
    pub padding_y: f32,
}

/// Styling applied to the header row.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TableHeaderStyle {
    pub fill_color: Option<String>,
    pub border_color: Option<String>,
    pub text_color: Option<String>,
}

fn default_font_size() -> f32 {
    12.0
}

fn default_stroke_width() -> f32 {
    0.5
}

fn default_qr_kind() -> String {
    "epc".to_string()
}

fn default_qr_width() -> f32 {
    120.0
}

fn default_barcode_symbology() -> String {
    "code128".to_string()
}

fn default_barcode_width() -> f32 {
    160.0
}

fn default_barcode_height() -> f32 {
    50.0
}

fn default_list_start() -> i64 {
    1
}

fn default_list_indent() -> f32 {
    18.0
}

fn default_true() -> bool {
    true
}

fn default_chart_kind() -> String {
    "bar".to_string()
}

fn default_chart_height() -> f32 {
    180.0
}

fn default_page_filter() -> PageFilter {
    PageFilter::Keyword("all".to_string())
}

fn default_watermark_angle() -> f32 {
    45.0
}

fn default_watermark_size() -> f32 {
    96.0
}

fn default_bold_weight() -> FontWeight {
    FontWeight::Bold
}

// --- case-insensitive enum deserializers ---

fn de_alignment<'de, D>(d: D) -> std::result::Result<Alignment, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let s = String::deserialize(d)?;
    Ok(parse_alignment(&s))
}

fn de_font_weight<'de, D>(d: D) -> std::result::Result<FontWeight, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let s = String::deserialize(d)?;
    Ok(parse_font_weight(&s))
}

pub(crate) fn de_opt_alignment<'de, D>(d: D) -> std::result::Result<Option<Alignment>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let s = Option::<String>::deserialize(d)?;
    Ok(s.map(|s| parse_alignment(&s)))
}

pub(crate) fn de_opt_font_weight<'de, D>(d: D) -> std::result::Result<Option<FontWeight>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let s = Option::<String>::deserialize(d)?;
    Ok(s.map(|s| parse_font_weight(&s)))
}

fn parse_alignment(s: &str) -> Alignment {
    match s.trim().to_ascii_lowercase().as_str() {
        "right" => Alignment::Right,
        "center" | "centre" => Alignment::Center,
        _ => Alignment::Left,
    }
}

fn parse_font_weight(s: &str) -> FontWeight {
    match s.trim().to_ascii_lowercase().as_str() {
        "bold" | "semibold" | "700" | "600" => FontWeight::Bold,
        _ => FontWeight::Normal,
    }
}
