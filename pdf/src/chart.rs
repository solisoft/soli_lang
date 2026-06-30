//! Chart rendering — bar, line and pie — composed entirely from the existing
//! draw primitives (filled rectangles, polygons, line segments, text). No extra
//! dependencies: a chart is just a recipe of [`DrawOp`]s laid out inside a box.

use crate::color::{self, Rgb};
use crate::draw::{DrawOp, PolyPoint, TextDraw, TextPiece};
use crate::error::RenderWarning;
use crate::fonts::FontRegistry;
use crate::template::{ChartEl, FontWeight};

/// The plot box for a chart (logical coords, top-left origin, points).
#[derive(Clone, Copy)]
pub struct Area {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

/// Fallback categorical palette (hex, no `#`) cycled when the element gives none.
const PALETTE: [&str; 8] = [
    "3b82f6", "ef4444", "10b981", "f59e0b", "8b5cf6", "ec4899", "14b8a6", "6366f1",
];
/// Font size for axis/legend/value labels.
const LABEL_SIZE: f32 = 8.0;
/// Bottom band reserved for category labels under a bar/line plot.
const LABEL_BAND: f32 = 14.0;

/// Render a chart into `area`, returning the draw ops. `series` is the resolved
/// `(label, value)` pairs. Empty series yield no ops (the caller warns).
pub fn render_chart(
    chart: &ChartEl,
    series: &[(String, f64)],
    area: Area,
    fonts: &FontRegistry,
    warnings: &mut Vec<RenderWarning>,
) -> Vec<DrawOp> {
    if series.is_empty() {
        return Vec::new();
    }
    match chart.kind.trim().to_ascii_lowercase().as_str() {
        "pie" | "donut" => render_pie(chart, series, area, fonts, warnings),
        "line" => render_line(chart, series, area, fonts, warnings),
        _ => render_bar(chart, series, area, fonts, warnings),
    }
}

fn axis_color() -> Rgb {
    color::parse_hex("9ca3af").unwrap_or(Rgb::LIGHT_GREY)
}

fn palette_color(colors: &[String], i: usize) -> Rgb {
    if !colors.is_empty() {
        if let Some(c) = colors
            .get(i % colors.len())
            .and_then(|s| color::parse_hex(s))
        {
            return c;
        }
    }
    color::parse_hex(PALETTE[i % PALETTE.len()]).unwrap_or(Rgb::BLACK)
}

/// Format a numeric value compactly: integers without a fraction, else 1 dp.
fn fmt_value(v: f64) -> String {
    if (v - v.round()).abs() < 1e-9 {
        format!("{}", v.round() as i64)
    } else {
        format!("{v:.1}")
    }
}

/// A text op whose horizontal center is `center_x` at the given `baseline`.
fn centered_text(
    fonts: &FontRegistry,
    warnings: &mut Vec<RenderWarning>,
    text: &str,
    center_x: f32,
    baseline: f32,
    size: f32,
    color: Rgb,
) -> Option<DrawOp> {
    let runs = fonts.itemize(text, FontWeight::Normal, warnings);
    let width: f32 = runs.iter().map(|r| fonts.measure_run(r, size)).sum();
    if runs.is_empty() {
        return None;
    }
    let pieces = runs
        .into_iter()
        .map(|r| TextPiece {
            slot: r.slot,
            text: r.text,
        })
        .collect();
    Some(DrawOp::Text(TextDraw {
        x: center_x - width / 2.0,
        baseline,
        size,
        color,
        pieces,
    }))
}

/// A left-aligned text op with its left edge at `x`.
fn left_text(
    fonts: &FontRegistry,
    warnings: &mut Vec<RenderWarning>,
    text: &str,
    x: f32,
    baseline: f32,
    size: f32,
    color: Rgb,
) -> Option<DrawOp> {
    let runs = fonts.itemize(text, FontWeight::Normal, warnings);
    if runs.is_empty() {
        return None;
    }
    let pieces = runs
        .into_iter()
        .map(|r| TextPiece {
            slot: r.slot,
            text: r.text,
        })
        .collect();
    Some(DrawOp::Text(TextDraw {
        x,
        baseline,
        size,
        color,
        pieces,
    }))
}

fn render_bar(
    chart: &ChartEl,
    series: &[(String, f64)],
    area: Area,
    fonts: &FontRegistry,
    warnings: &mut Vec<RenderWarning>,
) -> Vec<DrawOp> {
    let mut ops = Vec::new();
    let n = series.len();
    let max = series
        .iter()
        .map(|(_, v)| *v)
        .fold(0.0_f64, f64::max)
        .max(f64::EPSILON);

    let plot_h = (area.h - LABEL_BAND).max(1.0);
    let baseline_y = area.y + plot_h;
    let slot = area.w / n as f32;
    let bar_w = (slot * 0.62).max(1.0);
    let ascent = fonts.ascent(FontWeight::Normal) * LABEL_SIZE;

    for (i, (label, v)) in series.iter().enumerate() {
        let frac = (v / max).clamp(0.0, 1.0) as f32;
        let bh = frac * plot_h;
        let bx = area.x + i as f32 * slot + (slot - bar_w) / 2.0;
        let by = baseline_y - bh;
        ops.push(DrawOp::FillRect {
            x: bx,
            y: by,
            w: bar_w,
            h: bh,
            color: palette_color(&chart.colors, i),
        });
        // Value label above the bar.
        if let Some(op) = centered_text(
            fonts,
            warnings,
            &fmt_value(*v),
            bx + bar_w / 2.0,
            (by - 2.0).max(area.y + ascent),
            LABEL_SIZE,
            Rgb::BLACK,
        ) {
            ops.push(op);
        }
        // Category label below the axis.
        if chart.axis {
            if let Some(op) = centered_text(
                fonts,
                warnings,
                label,
                bx + bar_w / 2.0,
                baseline_y + ascent + 2.0,
                LABEL_SIZE,
                axis_color(),
            ) {
                ops.push(op);
            }
        }
    }

    if chart.axis {
        let ax = axis_color();
        ops.push(hline(area.x, baseline_y, area.x + area.w, ax));
        ops.push(vline(area.x, area.y, baseline_y, ax));
    }
    ops
}

fn render_line(
    chart: &ChartEl,
    series: &[(String, f64)],
    area: Area,
    fonts: &FontRegistry,
    warnings: &mut Vec<RenderWarning>,
) -> Vec<DrawOp> {
    let mut ops = Vec::new();
    let n = series.len();
    let max = series
        .iter()
        .map(|(_, v)| *v)
        .fold(0.0_f64, f64::max)
        .max(f64::EPSILON);

    let plot_h = (area.h - LABEL_BAND).max(1.0);
    let baseline_y = area.y + plot_h;
    let ascent = fonts.ascent(FontWeight::Normal) * LABEL_SIZE;
    let color = palette_color(&chart.colors, 0);

    // Point coordinates, spread edge to edge (or centered for a single point).
    let xs: Vec<f32> = if n == 1 {
        vec![area.x + area.w / 2.0]
    } else {
        (0..n)
            .map(|i| area.x + (i as f32 / (n - 1) as f32) * area.w)
            .collect()
    };
    let pts: Vec<(f32, f32)> = series
        .iter()
        .enumerate()
        .map(|(i, (_, v))| {
            let frac = (v / max).clamp(0.0, 1.0) as f32;
            (xs[i], baseline_y - frac * plot_h)
        })
        .collect();

    // Connect consecutive points with line segments (an open polyline — not a
    // closed Polygon, which would draw a spurious closing edge).
    for w in pts.windows(2) {
        ops.push(DrawOp::Line {
            x1: w[0].0,
            y1: w[0].1,
            x2: w[1].0,
            y2: w[1].1,
            width: 1.5,
            color,
            dash: None,
        });
    }
    // Point markers + labels.
    for (i, (label, _)) in series.iter().enumerate() {
        let (px, py) = pts[i];
        ops.push(DrawOp::Polygon {
            points: circle_points(px, py, 2.2),
            fill: Some(color),
            stroke: None,
            stroke_width: 0.0,
            dash: None,
        });
        if chart.axis {
            if let Some(op) = centered_text(
                fonts,
                warnings,
                label,
                px,
                baseline_y + ascent + 2.0,
                LABEL_SIZE,
                axis_color(),
            ) {
                ops.push(op);
            }
        }
    }

    if chart.axis {
        let ax = axis_color();
        ops.push(hline(area.x, baseline_y, area.x + area.w, ax));
        ops.push(vline(area.x, area.y, baseline_y, ax));
    }
    ops
}

fn render_pie(
    chart: &ChartEl,
    series: &[(String, f64)],
    area: Area,
    fonts: &FontRegistry,
    warnings: &mut Vec<RenderWarning>,
) -> Vec<DrawOp> {
    let mut ops = Vec::new();
    let total: f64 = series.iter().map(|(_, v)| v.max(0.0)).sum();
    if total <= 0.0 {
        return ops;
    }

    let legend_w = if chart.legend {
        (area.w * 0.4).min(160.0)
    } else {
        0.0
    };
    let pie_w = (area.w - legend_w).max(1.0);
    let radius = (pie_w.min(area.h) / 2.0 - 4.0).max(1.0);
    let cx = area.x + pie_w / 2.0;
    let cy = area.y + area.h / 2.0;
    let white = color::parse_hex("ffffff").unwrap_or(Rgb::LIGHT_GREY);

    let mut angle = -90.0_f32; // start at the top
    for (i, (_, v)) in series.iter().enumerate() {
        let sweep = (v.max(0.0) / total) as f32 * 360.0;
        let next = angle + sweep;
        ops.push(DrawOp::Polygon {
            points: wedge_points(cx, cy, radius, angle, next),
            fill: Some(palette_color(&chart.colors, i)),
            stroke: Some(white),
            stroke_width: 1.0,
            dash: None,
        });
        angle = next;
    }

    if chart.legend {
        let ascent = fonts.ascent(FontWeight::Normal) * LABEL_SIZE;
        let lx = area.x + pie_w + 6.0;
        let swatch = 9.0;
        let row_h = 14.0;
        let mut ly = area.y + 2.0;
        for (i, (label, v)) in series.iter().enumerate() {
            ops.push(DrawOp::FillRect {
                x: lx,
                y: ly,
                w: swatch,
                h: swatch,
                color: palette_color(&chart.colors, i),
            });
            let pct = v.max(0.0) / total * 100.0;
            let text = format!("{label}  {}%", fmt_value(pct));
            if let Some(op) = left_text(
                fonts,
                warnings,
                &text,
                lx + swatch + 4.0,
                ly + ascent,
                LABEL_SIZE,
                Rgb::BLACK,
            ) {
                ops.push(op);
            }
            ly += row_h;
        }
    }
    ops
}

fn hline(x1: f32, y: f32, x2: f32, color: Rgb) -> DrawOp {
    DrawOp::Line {
        x1,
        y1: y,
        x2,
        y2: y,
        width: 0.8,
        color,
        dash: None,
    }
}

fn vline(x: f32, y1: f32, y2: f32, color: Rgb) -> DrawOp {
    DrawOp::Line {
        x1: x,
        y1,
        x2: x,
        y2,
        width: 0.8,
        color,
        dash: None,
    }
}

/// A small filled disc approximated by a polygon, for line-chart point markers.
fn circle_points(cx: f32, cy: f32, r: f32) -> Vec<PolyPoint> {
    wedge_arc(cx, cy, r, 0.0, 360.0, false)
}

/// A pie wedge: center → arc → (path closes back to center). Straight segments
/// approximate the arc finely enough to look smooth at print sizes.
fn wedge_points(cx: f32, cy: f32, r: f32, start_deg: f32, end_deg: f32) -> Vec<PolyPoint> {
    let mut pts = vec![PolyPoint {
        x: cx,
        y: cy,
        bezier: false,
    }];
    pts.extend(wedge_arc(cx, cy, r, start_deg, end_deg, true));
    pts
}

/// Sample points along an arc from `start_deg` to `end_deg` (logical coords).
/// `include_start` keeps the first arc point (omitted by wedge, whose first
/// point is the center).
fn wedge_arc(
    cx: f32,
    cy: f32,
    r: f32,
    start_deg: f32,
    end_deg: f32,
    include_start: bool,
) -> Vec<PolyPoint> {
    let span = (end_deg - start_deg).abs();
    let steps = (span / 4.0).ceil().max(1.0) as usize;
    let mut pts = Vec::with_capacity(steps + 1);
    let from = if include_start { 0 } else { 1 };
    for i in from..=steps {
        let t = start_deg + (end_deg - start_deg) * (i as f32 / steps as f32);
        let a = t.to_radians();
        pts.push(PolyPoint {
            x: cx + r * a.cos(),
            y: cy + r * a.sin(),
            bezier: false,
        });
    }
    pts
}

#[cfg(test)]
mod tests {
    use super::*;

    fn area() -> Area {
        Area {
            x: 0.0,
            y: 0.0,
            w: 300.0,
            h: 180.0,
        }
    }

    fn fonts() -> FontRegistry {
        FontRegistry::from_font_dirs(
            &[std::path::PathBuf::from("fonts")],
            &["titillium".to_string()],
        )
        .unwrap()
    }

    fn series() -> Vec<(String, f64)> {
        vec![
            ("Jan".to_string(), 10.0),
            ("Feb".to_string(), 25.0),
            ("Mar".to_string(), 15.0),
        ]
    }

    #[test]
    fn bar_emits_one_fillrect_per_point() {
        let chart: ChartEl = serde_json::from_str(r#"{"kind":"bar"}"#).unwrap();
        let ops = render_chart(&chart, &series(), area(), &fonts(), &mut Vec::new());
        let bars = ops
            .iter()
            .filter(|op| matches!(op, DrawOp::FillRect { .. }))
            .count();
        assert_eq!(bars, 3);
    }

    #[test]
    fn pie_emits_one_polygon_per_slice() {
        let chart: ChartEl = serde_json::from_str(r#"{"kind":"pie","legend":false}"#).unwrap();
        let ops = render_chart(&chart, &series(), area(), &fonts(), &mut Vec::new());
        let slices = ops
            .iter()
            .filter(|op| matches!(op, DrawOp::Polygon { .. }))
            .count();
        assert_eq!(slices, 3);
    }

    #[test]
    fn line_connects_points_with_segments() {
        let chart: ChartEl = serde_json::from_str(r#"{"kind":"line"}"#).unwrap();
        let ops = render_chart(&chart, &series(), area(), &fonts(), &mut Vec::new());
        // 3 points → 2 connecting segments (plus 2 axis lines).
        let segments = ops
            .iter()
            .filter(|op| matches!(op, DrawOp::Line { .. }))
            .count();
        assert_eq!(segments, 2 + 2);
    }

    #[test]
    fn empty_series_draws_nothing() {
        let chart: ChartEl = serde_json::from_str(r#"{"kind":"bar"}"#).unwrap();
        let ops = render_chart(&chart, &[], area(), &fonts(), &mut Vec::new());
        assert!(ops.is_empty());
    }
}
