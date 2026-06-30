//! Page geometry and the top-left ↔ PDF (bottom-left) coordinate transform.
//!
//! The engine works in a **top-left origin, y-increases-downward** space (pt).
//! A `move` element adds its delta directly to the cursor, so a negative
//! `move.y` goes UP and a positive `move.y` goes DOWN — matching the template's
//! logo placement. Conversion to PDF's native bottom-left space happens in
//! exactly one place: [`Page::to_pdf_y`].

/// A4 portrait dimensions in points.
pub const A4_WIDTH_PT: f32 = 595.276;
pub const A4_HEIGHT_PT: f32 = 841.890;

/// 20 mm in points (default page margin).
pub const DEFAULT_MARGIN_PT: f32 = 56.693;

/// Resolve a named page size to portrait `(width, height)` in points. Unknown
/// names fall back to A4.
pub fn named_page_size(name: &str) -> (f32, f32) {
    match name.trim().to_ascii_lowercase().as_str() {
        "letter" => (612.0, 792.0),
        "legal" => (612.0, 1008.0),
        "a5" => (420.945, 595.276),
        "a3" => (841.890, 1190.551),
        _ => (A4_WIDTH_PT, A4_HEIGHT_PT),
    }
}

/// Page margins in points.
#[derive(Debug, Clone, Copy)]
pub struct Margins {
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
    pub left: f32,
}

impl Default for Margins {
    fn default() -> Self {
        Margins {
            top: DEFAULT_MARGIN_PT,
            right: DEFAULT_MARGIN_PT,
            bottom: DEFAULT_MARGIN_PT,
            left: DEFAULT_MARGIN_PT,
        }
    }
}

/// The page model: size, margins, and the reserved header/footer bands.
#[derive(Debug, Clone, Copy)]
pub struct Page {
    pub width: f32,
    pub height: f32,
    pub margins: Margins,
    /// Reserved band at the top (below the top margin) for the header.
    pub header_height: f32,
    /// Reserved band at the bottom (above the bottom margin) for the footer.
    pub footer_height: f32,
}

impl Default for Page {
    fn default() -> Self {
        Page {
            width: A4_WIDTH_PT,
            height: A4_HEIGHT_PT,
            margins: Margins::default(),
            header_height: 0.0,
            footer_height: 0.0,
        }
    }
}

impl Page {
    /// Left edge of the content region (logical x).
    pub fn content_left(&self) -> f32 {
        self.margins.left
    }

    /// Right edge of the content region (logical x).
    pub fn content_right(&self) -> f32 {
        self.width - self.margins.right
    }

    /// Usable content width.
    pub fn content_width(&self) -> f32 {
        (self.width - self.margins.left - self.margins.right).max(0.0)
    }

    /// Top of the content region in logical (top-down) coordinates: below the
    /// top margin and the header band.
    pub fn content_top(&self) -> f32 {
        self.margins.top + self.header_height
    }

    /// Bottom of the content region in logical coordinates: above the bottom
    /// margin and the footer band.
    pub fn content_bottom(&self) -> f32 {
        self.height - self.margins.bottom - self.footer_height
    }

    /// Logical-y of the top of the footer band.
    pub fn footer_top(&self) -> f32 {
        self.height - self.margins.bottom - self.footer_height
    }

    /// Logical-y of the top of the header band (= top margin).
    pub fn header_top(&self) -> f32 {
        self.margins.top
    }

    /// Convert a logical (top-down) y to PDF (bottom-up) y.
    pub fn to_pdf_y(&self, logical_y: f32) -> f32 {
        self.height - logical_y
    }
}

/// The layout cursor, in logical (top-left) coordinates.
#[derive(Debug, Clone, Copy)]
pub struct Cursor {
    pub x: f32,
    pub y: f32,
}

impl Cursor {
    pub fn new(x: f32, y: f32) -> Self {
        Cursor { x, y }
    }

    /// Apply a relative move (positive y = down, negative y = up).
    pub fn move_by(&mut self, dx: f32, dy: f32) {
        self.x += dx;
        self.y += dy;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pdf_y_flips_origin() {
        let p = Page::default();
        // Top of page (logical 0) maps to the full height in PDF space.
        assert!((p.to_pdf_y(0.0) - A4_HEIGHT_PT).abs() < 1e-3);
        // Logical 100 down from the top.
        assert!((p.to_pdf_y(100.0) - (A4_HEIGHT_PT - 100.0)).abs() < 1e-3);
    }

    #[test]
    fn move_sign_convention() {
        let mut c = Cursor::new(100.0, 200.0);
        c.move_by(412.0, -30.0); // right + up
        assert_eq!((c.x, c.y), (512.0, 170.0));
        c.move_by(-412.0, -60.0); // left + further up
        assert_eq!((c.x, c.y), (100.0, 110.0));
    }

    #[test]
    fn content_region() {
        let p = Page {
            header_height: 40.0,
            footer_height: 20.0,
            ..Page::default()
        };
        assert!((p.content_top() - (DEFAULT_MARGIN_PT + 40.0)).abs() < 1e-3);
        assert!((p.content_bottom() - (A4_HEIGHT_PT - DEFAULT_MARGIN_PT - 20.0)).abs() < 1e-3);
    }
}
