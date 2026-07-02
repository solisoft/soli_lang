//! Chart rendering — bar, line and pie — composed entirely from the existing
//! draw primitives (filled rectangles, polygons, line segments, text). No extra
//! dependencies: a chart is just a recipe of [`DrawOp`]s laid out inside a box.
//!
//! Charts are single- or multi-series. A single series keeps the original look
//! (one bar/slice per category, colored from the palette by category, with a
//! value label above each bar). Multiple series add grouped/stacked bars or
//! multiple lines, a shared legend, and — opt-in — value-axis gridlines.

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

/// One resolved chart series: an optional legend `name` and `color` (hex, no
/// `#`), plus one value per category.
#[derive(Debug, Clone)]
pub struct Series {
    pub name: Option<String>,
    pub color: Option<String>,
    pub values: Vec<f64>,
}

/// Fallback categorical palette (hex, no `#`) cycled when the element gives none.
const PALETTE: [&str; 8] = [
    "3b82f6", "ef4444", "10b981", "f59e0b", "8b5cf6", "ec4899", "14b8a6", "6366f1",
];
/// Font size for axis/legend/value labels.
const LABEL_SIZE: f32 = 8.0;
/// Bottom band reserved for category labels under a bar/line plot.
const LABEL_BAND: f32 = 14.0;
/// Top band reserved for a multi-series legend.
const LEGEND_BAND: f32 = 16.0;

/// Render a chart into `area`, returning the draw ops. `categories` are the
/// x-axis labels; `series` holds one or more value rows aligned to them. Empty
/// input yields no ops (the caller warns).
pub fn render_chart(
    chart: &ChartEl,
    categories: &[String],
    series: &[Series],
    area: Area,
    fonts: &FontRegistry,
    warnings: &mut Vec<RenderWarning>,
) -> Vec<DrawOp> {
    if categories.is_empty() || series.is_empty() {
        return Vec::new();
    }
    match chart.kind.trim().to_ascii_lowercase().as_str() {
        "pie" | "donut" => render_pie(chart, categories, &series[0], area, fonts, warnings),
        "line" => render_line(chart, categories, series, area, fonts, warnings),
        _ => render_bar(chart, categories, series, area, fonts, warnings),
    }
}

fn axis_color() -> Rgb {
    color::parse_hex("9ca3af").unwrap_or(Rgb::LIGHT_GREY)
}

fn grid_color() -> Rgb {
    color::parse_hex("e5e7eb").unwrap_or(Rgb::LIGHT_GREY)
}

/// Color for series `k`: its own `color` if set, else the palette by index.
fn series_color(chart: &ChartEl, s: &Series, k: usize) -> Rgb {
    if let Some(c) = s.color.as_deref().and_then(color::parse_hex) {
        return c;
    }
    palette_color(&chart.colors, k)
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

/// Round a positive max up to a "nice" axis ceiling (1/2/5 × 10ⁿ).
fn nice_max(max: f64) -> f64 {
    if max <= 0.0 {
        return 1.0;
    }
    let pow = 10f64.powf(max.log10().floor());
    let frac = max / pow;
    let nice = if frac <= 1.0 {
        1.0
    } else if frac <= 2.0 {
        2.0
    } else if frac <= 5.0 {
        5.0
    } else {
        10.0
    };
    nice * pow
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

/// A left-aligned text op with its left edge at `x`. Returns the op and the
/// measured text width (for laying out a horizontal legend).
fn left_text(
    fonts: &FontRegistry,
    warnings: &mut Vec<RenderWarning>,
    text: &str,
    x: f32,
    baseline: f32,
    size: f32,
    color: Rgb,
) -> Option<(DrawOp, f32)> {
    let runs = fonts.itemize(text, FontWeight::Normal, warnings);
    if runs.is_empty() {
        return None;
    }
    let width: f32 = runs.iter().map(|r| fonts.measure_run(r, size)).sum();
    let pieces = runs
        .into_iter()
        .map(|r| TextPiece {
            slot: r.slot,
            text: r.text,
        })
        .collect();
    Some((
        DrawOp::Text(TextDraw {
            x,
            baseline,
            size,
            color,
            pieces,
        }),
        width,
    ))
}

/// A text op whose right edge sits at `x` (used for value-axis tick labels).
fn right_text(
    fonts: &FontRegistry,
    warnings: &mut Vec<RenderWarning>,
    text: &str,
    x: f32,
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
        x: x - width,
        baseline,
        size,
        color,
        pieces,
    }))
}

/// Width to reserve at the left for value-axis tick labels.
fn value_gutter(fonts: &FontRegistry, scale: f64) -> f32 {
    let runs = fonts.itemize(&fmt_value(scale), FontWeight::Normal, &mut Vec::new());
    let w: f32 = runs.iter().map(|r| fonts.measure_run(r, LABEL_SIZE)).sum();
    w + 6.0
}

/// Horizontal value-axis gridlines (0 → `scale`, 4 intervals) plus right-aligned
/// tick labels in the left gutter.
#[allow(clippy::too_many_arguments)]
fn gridlines(
    plot_x: f32,
    plot_w: f32,
    plot_h: f32,
    baseline_y: f32,
    scale: f64,
    fonts: &FontRegistry,
    warnings: &mut Vec<RenderWarning>,
) -> Vec<DrawOp> {
    let mut ops = Vec::new();
    let ticks = 4;
    let gc = grid_color();
    let cap = fonts.cap_height(FontWeight::Normal) * LABEL_SIZE;
    for t in 0..=ticks {
        let frac = t as f32 / ticks as f32;
        let y = baseline_y - frac * plot_h;
        ops.push(DrawOp::Line {
            x1: plot_x,
            y1: y,
            x2: plot_x + plot_w,
            y2: y,
            width: 0.5,
            color: gc,
            dash: None,
        });
        if let Some(op) = right_text(
            fonts,
            warnings,
            &fmt_value(scale * frac as f64),
            plot_x - 3.0,
            y + cap / 2.0,
            LABEL_SIZE,
            axis_color(),
        ) {
            ops.push(op);
        }
    }
    ops
}

/// A horizontal legend row (swatch + name per series) starting at `(x, y)`.
fn legend_row(
    chart: &ChartEl,
    series: &[Series],
    x: f32,
    y: f32,
    fonts: &FontRegistry,
    warnings: &mut Vec<RenderWarning>,
) -> Vec<DrawOp> {
    let mut ops = Vec::new();
    let ascent = fonts.ascent(FontWeight::Normal) * LABEL_SIZE;
    let swatch = 8.0;
    let mut lx = x;
    for (k, s) in series.iter().enumerate() {
        let name = s
            .name
            .clone()
            .unwrap_or_else(|| format!("Series {}", k + 1));
        ops.push(DrawOp::FillRect {
            x: lx,
            y: y + 2.0,
            w: swatch,
            h: swatch,
            color: series_color(chart, s, k),
        });
        let tx = lx + swatch + 4.0;
        if let Some((op, w)) = left_text(
            fonts,
            warnings,
            &name,
            tx,
            y + ascent,
            LABEL_SIZE,
            Rgb::BLACK,
        ) {
            ops.push(op);
            lx = tx + w + 14.0;
        } else {
            lx = tx + 14.0;
        }
    }
    ops
}

fn render_bar(
    chart: &ChartEl,
    categories: &[String],
    series: &[Series],
    area: Area,
    fonts: &FontRegistry,
    warnings: &mut Vec<RenderWarning>,
) -> Vec<DrawOp> {
    let mut ops = Vec::new();
    let n = categories.len();
    let s = series.len();
    let multi = s > 1;
    let stacked = multi
        && chart
            .mode
            .as_deref()
            .is_some_and(|m| m.eq_ignore_ascii_case("stacked"));

    // Scale: stacked sums per category; otherwise the global max. With gridlines
    // the ceiling is rounded to a nice value; without (the default), the tallest
    // bar fills the plot — preserving the original single-series look.
    let raw_max = if stacked {
        (0..n)
            .map(|i| {
                series
                    .iter()
                    .map(|se| se.values.get(i).copied().unwrap_or(0.0).max(0.0))
                    .sum::<f64>()
            })
            .fold(0.0_f64, f64::max)
    } else {
        series
            .iter()
            .flat_map(|se| se.values.iter().copied())
            .fold(0.0_f64, f64::max)
    };
    let scale = if chart.gridlines {
        nice_max(raw_max)
    } else {
        raw_max.max(f64::EPSILON)
    };

    let legend_h = if multi && chart.legend {
        LEGEND_BAND
    } else {
        0.0
    };
    let gutter_w = if chart.gridlines {
        value_gutter(fonts, scale)
    } else {
        0.0
    };
    let plot_x = area.x + gutter_w;
    let plot_y = area.y + legend_h;
    let plot_w = (area.w - gutter_w).max(1.0);
    let plot_h = (area.h - legend_h - LABEL_BAND).max(1.0);
    let baseline_y = plot_y + plot_h;
    let ascent = fonts.ascent(FontWeight::Normal) * LABEL_SIZE;

    if chart.gridlines {
        ops.extend(gridlines(
            plot_x, plot_w, plot_h, baseline_y, scale, fonts, warnings,
        ));
    }

    let slot = plot_w / n as f32;
    for (i, category) in categories.iter().enumerate() {
        let gx = plot_x + i as f32 * slot;
        if stacked {
            let bar_w = (slot * 0.62).max(1.0);
            let bx = gx + (slot - bar_w) / 2.0;
            let mut acc = 0.0_f64;
            for (k, se) in series.iter().enumerate() {
                let v = se.values.get(i).copied().unwrap_or(0.0).max(0.0);
                if v <= 0.0 {
                    continue;
                }
                let seg_h = (v / scale) as f32 * plot_h;
                let y = baseline_y - ((acc + v) / scale) as f32 * plot_h;
                ops.push(DrawOp::FillRect {
                    x: bx,
                    y,
                    w: bar_w,
                    h: seg_h,
                    color: series_color(chart, se, k),
                });
                acc += v;
            }
        } else if multi {
            let gpad = slot * 0.15;
            let inner = (slot - 2.0 * gpad).max(1.0);
            let step = inner / s as f32;
            let bar_w = (step * 0.84).max(1.0);
            for (k, se) in series.iter().enumerate() {
                let v = se.values.get(i).copied().unwrap_or(0.0).max(0.0);
                let bh = (v / scale).clamp(0.0, 1.0) as f32 * plot_h;
                let bx = gx + gpad + k as f32 * step + (step - bar_w) / 2.0;
                ops.push(DrawOp::FillRect {
                    x: bx,
                    y: baseline_y - bh,
                    w: bar_w,
                    h: bh,
                    color: series_color(chart, se, k),
                });
            }
        } else {
            // Single series: original look — one bar per category, colored from
            // the palette by category, with a value label above.
            let v = series[0].values.get(i).copied().unwrap_or(0.0);
            let bar_w = (slot * 0.62).max(1.0);
            let bh = (v.max(0.0) / scale).clamp(0.0, 1.0) as f32 * plot_h;
            let bx = gx + (slot - bar_w) / 2.0;
            let by = baseline_y - bh;
            ops.push(DrawOp::FillRect {
                x: bx,
                y: by,
                w: bar_w,
                h: bh,
                color: palette_color(&chart.colors, i),
            });
            if let Some(op) = centered_text(
                fonts,
                warnings,
                &fmt_value(v),
                bx + bar_w / 2.0,
                (by - 2.0).max(plot_y + ascent),
                LABEL_SIZE,
                Rgb::BLACK,
            ) {
                ops.push(op);
            }
        }
        // Category label below the axis.
        if chart.axis {
            if let Some(op) = centered_text(
                fonts,
                warnings,
                category,
                gx + slot / 2.0,
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
        ops.push(hline(plot_x, baseline_y, plot_x + plot_w, ax));
        ops.push(vline(plot_x, plot_y, baseline_y, ax));
    }
    if multi && chart.legend {
        ops.extend(legend_row(chart, series, plot_x, area.y, fonts, warnings));
    }
    ops
}

fn render_line(
    chart: &ChartEl,
    categories: &[String],
    series: &[Series],
    area: Area,
    fonts: &FontRegistry,
    warnings: &mut Vec<RenderWarning>,
) -> Vec<DrawOp> {
    let mut ops = Vec::new();
    let n = categories.len();
    let multi = series.len() > 1;
    let raw_max = series
        .iter()
        .flat_map(|se| se.values.iter().copied())
        .fold(0.0_f64, f64::max);
    let scale = if chart.gridlines {
        nice_max(raw_max)
    } else {
        raw_max.max(f64::EPSILON)
    };

    let legend_h = if multi && chart.legend {
        LEGEND_BAND
    } else {
        0.0
    };
    let gutter_w = if chart.gridlines {
        value_gutter(fonts, scale)
    } else {
        0.0
    };
    let plot_x = area.x + gutter_w;
    let plot_y = area.y + legend_h;
    let plot_w = (area.w - gutter_w).max(1.0);
    let plot_h = (area.h - legend_h - LABEL_BAND).max(1.0);
    let baseline_y = plot_y + plot_h;
    let ascent = fonts.ascent(FontWeight::Normal) * LABEL_SIZE;

    if chart.gridlines {
        ops.extend(gridlines(
            plot_x, plot_w, plot_h, baseline_y, scale, fonts, warnings,
        ));
    }

    // Point x positions, spread edge to edge (or centered for a single point).
    let xs: Vec<f32> = if n == 1 {
        vec![plot_x + plot_w / 2.0]
    } else {
        (0..n)
            .map(|i| plot_x + (i as f32 / (n - 1) as f32) * plot_w)
            .collect()
    };

    for (k, se) in series.iter().enumerate() {
        let color = series_color(chart, se, k);
        let pts: Vec<(f32, f32)> = (0..n)
            .map(|i| {
                let v = se.values.get(i).copied().unwrap_or(0.0);
                let frac = (v.max(0.0) / scale).clamp(0.0, 1.0) as f32;
                (xs[i], baseline_y - frac * plot_h)
            })
            .collect();
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
        for &(px, py) in &pts {
            ops.push(DrawOp::Polygon {
                points: circle_points(px, py, 2.2),
                fill: Some(color),
                stroke: None,
                stroke_width: 0.0,
                dash: None,
            });
        }
    }

    if chart.axis {
        for (i, label) in categories.iter().enumerate() {
            if let Some(op) = centered_text(
                fonts,
                warnings,
                label,
                xs[i],
                baseline_y + ascent + 2.0,
                LABEL_SIZE,
                axis_color(),
            ) {
                ops.push(op);
            }
        }
        let ax = axis_color();
        ops.push(hline(plot_x, baseline_y, plot_x + plot_w, ax));
        ops.push(vline(plot_x, plot_y, baseline_y, ax));
    }
    if multi && chart.legend {
        ops.extend(legend_row(chart, series, plot_x, area.y, fonts, warnings));
    }
    ops
}

fn render_pie(
    chart: &ChartEl,
    categories: &[String],
    series: &Series,
    area: Area,
    fonts: &FontRegistry,
    warnings: &mut Vec<RenderWarning>,
) -> Vec<DrawOp> {
    let mut ops = Vec::new();
    let total: f64 = series.values.iter().map(|v| v.max(0.0)).sum();
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
    // A donut is the same wedges with a ring cutout: annular segments, so
    // whatever is behind the chart shows through the hole.
    let inner = if chart.kind.trim().eq_ignore_ascii_case("donut") {
        Some(radius * 0.55)
    } else {
        None
    };

    let mut angle = -90.0_f32; // start at the top
    for (i, v) in series.values.iter().enumerate() {
        let sweep = (v.max(0.0) / total) as f32 * 360.0;
        let next = angle + sweep;
        let points = match inner {
            Some(r_in) => annular_wedge_points(cx, cy, radius, r_in, angle, next),
            None => wedge_points(cx, cy, radius, angle, next),
        };
        ops.push(DrawOp::Polygon {
            points,
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
        for (i, v) in series.values.iter().enumerate() {
            ops.push(DrawOp::FillRect {
                x: lx,
                y: ly,
                w: swatch,
                h: swatch,
                color: palette_color(&chart.colors, i),
            });
            let label = categories.get(i).map(String::as_str).unwrap_or("");
            let pct = v.max(0.0) / total * 100.0;
            let text = format!("{label}  {}%", fmt_value(pct));
            if let Some((op, _)) = left_text(
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

/// An annular (donut) wedge: outer arc forward, then inner arc back, closed.
fn annular_wedge_points(
    cx: f32,
    cy: f32,
    r_outer: f32,
    r_inner: f32,
    start_deg: f32,
    end_deg: f32,
) -> Vec<PolyPoint> {
    let mut pts = wedge_arc(cx, cy, r_outer, start_deg, end_deg, true);
    pts.extend(wedge_arc(cx, cy, r_inner, end_deg, start_deg, true));
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

    fn cats() -> Vec<String> {
        vec!["Jan".to_string(), "Feb".to_string(), "Mar".to_string()]
    }

    fn one_series() -> Vec<Series> {
        vec![Series {
            name: None,
            color: None,
            values: vec![10.0, 25.0, 15.0],
        }]
    }

    #[test]
    fn bar_emits_one_fillrect_per_point() {
        let chart: ChartEl = serde_json::from_str(r#"{"kind":"bar"}"#).unwrap();
        let ops = render_chart(
            &chart,
            &cats(),
            &one_series(),
            area(),
            &fonts(),
            &mut Vec::new(),
        );
        let bars = ops
            .iter()
            .filter(|op| matches!(op, DrawOp::FillRect { .. }))
            .count();
        assert_eq!(bars, 3);
    }

    #[test]
    fn grouped_bar_emits_one_fillrect_per_series_value() {
        let chart: ChartEl = serde_json::from_str(r#"{"kind":"bar","legend":false}"#).unwrap();
        let series = vec![
            Series {
                name: Some("A".into()),
                color: None,
                values: vec![1.0, 2.0, 3.0],
            },
            Series {
                name: Some("B".into()),
                color: None,
                values: vec![4.0, 5.0, 6.0],
            },
        ];
        let ops = render_chart(&chart, &cats(), &series, area(), &fonts(), &mut Vec::new());
        let bars = ops
            .iter()
            .filter(|op| matches!(op, DrawOp::FillRect { .. }))
            .count();
        assert_eq!(bars, 6, "2 series × 3 categories");
    }

    #[test]
    fn pie_emits_one_polygon_per_slice() {
        let chart: ChartEl = serde_json::from_str(r#"{"kind":"pie","legend":false}"#).unwrap();
        let ops = render_chart(
            &chart,
            &cats(),
            &one_series(),
            area(),
            &fonts(),
            &mut Vec::new(),
        );
        let slices = ops
            .iter()
            .filter(|op| matches!(op, DrawOp::Polygon { .. }))
            .count();
        assert_eq!(slices, 3);
    }

    #[test]
    fn donut_renders_annular_wedges() {
        let chart: ChartEl = serde_json::from_str(r#"{"kind":"donut","legend":false}"#).unwrap();
        let ops = render_chart(
            &chart,
            &cats(),
            &one_series(),
            area(),
            &fonts(),
            &mut Vec::new(),
        );
        // Area 300x180, no legend → center (150, 90). A true donut has a ring
        // cutout: no wedge point may sit at (or near) the center, unlike a pie
        // wedge whose first point IS the center.
        let mut slices = 0;
        for op in &ops {
            if let DrawOp::Polygon { points, .. } = op {
                slices += 1;
                for p in points {
                    let d = ((p.x - 150.0).powi(2) + (p.y - 90.0).powi(2)).sqrt();
                    assert!(d > 10.0, "point ({}, {}) too close to the center", p.x, p.y);
                }
            }
        }
        assert_eq!(slices, 3);
    }

    #[test]
    fn line_connects_points_with_segments() {
        let chart: ChartEl = serde_json::from_str(r#"{"kind":"line"}"#).unwrap();
        let ops = render_chart(
            &chart,
            &cats(),
            &one_series(),
            area(),
            &fonts(),
            &mut Vec::new(),
        );
        // 3 points → 2 connecting segments (plus 2 axis lines).
        let segments = ops
            .iter()
            .filter(|op| matches!(op, DrawOp::Line { .. }))
            .count();
        assert_eq!(segments, 2 + 2);
    }

    #[test]
    fn gridlines_add_horizontal_lines() {
        let chart: ChartEl =
            serde_json::from_str(r#"{"kind":"bar","gridlines":true,"axis":false}"#).unwrap();
        let ops = render_chart(
            &chart,
            &cats(),
            &one_series(),
            area(),
            &fonts(),
            &mut Vec::new(),
        );
        // 5 gridlines (0..=4 ticks), no axis lines.
        let lines = ops
            .iter()
            .filter(|op| matches!(op, DrawOp::Line { .. }))
            .count();
        assert_eq!(lines, 5);
    }

    #[test]
    fn empty_series_draws_nothing() {
        let chart: ChartEl = serde_json::from_str(r#"{"kind":"bar"}"#).unwrap();
        let ops = render_chart(&chart, &[], &[], area(), &fonts(), &mut Vec::new());
        assert!(ops.is_empty());
    }
}
