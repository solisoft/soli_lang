//! Table cell model.
//!
//! A cell is either a simple **text** cell (`{ "text": "...", ... }`) or a
//! **rich** cell (`{ "content": [ ... ], ... }`) whose `content` is a list of
//! inline items — text *or* image (or future types). The untagged enum tries
//! `Rich` first (it requires `content`), then falls back to `Text`.

use serde::Deserialize;

use super::{Alignment, FontWeight, ImageEl};

/// One table cell.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum Cell {
    /// A cell whose body is a vertical stack of inline items (text/image/...).
    Rich(RichCell),
    /// A cell whose body is a single (interpolatable) text string.
    Text(TextCell),
}

/// Vertical alignment of a cell's content within its row.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum VAlign {
    Top,
    /// The default: optically centered in the row.
    #[default]
    Middle,
    Bottom,
}

/// Shared per-cell styling (borders, width, alignment).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CellStyle {
    pub width: Option<f32>,
    pub font_size: Option<f32>,
    #[serde(default, deserialize_with = "super::de_opt_alignment")]
    pub alignment: Option<Alignment>,
    /// Vertical alignment within the row: `top` / `middle` (default) / `bottom`.
    #[serde(default, deserialize_with = "super::de_opt_valign")]
    pub valign: Option<VAlign>,
    #[serde(default, deserialize_with = "super::de_opt_font_weight")]
    pub font_weight: Option<FontWeight>,
    pub border_sides: Option<BorderSides>,
    pub border_color: Option<String>,
    /// Optional external URL; when set, the cell's text becomes a clickable link.
    pub link: Option<String>,
    /// Background fill for this cell (hex, no `#`), painted before borders and
    /// text. Wins over the table's zebra `stripe` and the header band fill.
    pub fill: Option<String>,
    /// Number of column slots this cell spans (default 1). The cell's box runs
    /// from its first column's left edge across the summed widths; following
    /// cells in the row shift right accordingly.
    pub colspan: Option<u32>,
}

/// A simple text cell.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextCell {
    pub text: String,
    #[serde(flatten)]
    pub style: CellStyle,
}

/// A rich cell with a stack of inline items.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RichCell {
    pub content: Vec<CellContent>,
    #[serde(flatten)]
    pub style: CellStyle,
}

/// One inline item inside a rich cell.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum CellContent {
    /// A line of text.
    Text(RichText),
    /// An inline image (fetched + scaled to `width`).
    Image(ImageEl),
}

/// A text run inside a rich cell.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RichText {
    pub value: String,
    pub font_size: Option<f32>,
    #[serde(default, deserialize_with = "super::de_opt_font_weight")]
    pub font_weight: Option<FontWeight>,
}

/// Which sides of a cell get a border line.
///
/// Values in the template are JSON strings (`"true"`/`"false"`) but plain
/// booleans are also accepted. When a `borderSides` object is present, any side
/// it omits defaults to `true` (so e.g. a header row that only lists
/// `right/left/top:false` still gets its bottom underline). When the whole
/// `borderSides` key is absent on a cell, no borders are drawn.
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BorderSides {
    #[serde(default = "bool_true", deserialize_with = "de_loose_bool")]
    pub right: bool,
    #[serde(default = "bool_true", deserialize_with = "de_loose_bool")]
    pub left: bool,
    #[serde(default = "bool_true", deserialize_with = "de_loose_bool")]
    pub top: bool,
    #[serde(default = "bool_true", deserialize_with = "de_loose_bool")]
    pub bottom: bool,
}

impl Default for BorderSides {
    fn default() -> Self {
        BorderSides {
            right: true,
            left: true,
            top: true,
            bottom: true,
        }
    }
}

impl BorderSides {
    /// All sides off — used when a cell omits `borderSides` entirely.
    pub const NONE: BorderSides = BorderSides {
        right: false,
        left: false,
        top: false,
        bottom: false,
    };

    pub fn any(&self) -> bool {
        self.right || self.left || self.top || self.bottom
    }
}

fn bool_true() -> bool {
    true
}

/// Accept a JSON bool or the strings `"true"`/`"false"` (case-insensitive).
fn de_loose_bool<'de, D>(d: D) -> std::result::Result<bool, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum BoolOrStr {
        Bool(bool),
        Str(String),
    }
    Ok(match BoolOrStr::deserialize(d)? {
        BoolOrStr::Bool(b) => b,
        BoolOrStr::Str(s) => !matches!(s.trim().to_ascii_lowercase().as_str(), "false" | "0" | ""),
    })
}

impl Cell {
    pub fn style(&self) -> &CellStyle {
        match self {
            Cell::Rich(c) => &c.style,
            Cell::Text(c) => &c.style,
        }
    }
}
