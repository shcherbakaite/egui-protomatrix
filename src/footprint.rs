//! KiCad footprint library reading and drawing.
//!
//! Loads `.kicad_mod` files via the in-tree kicad9 library and renders them to an egui
//! painter with layer filtering and coordinate transform (KiCad y-up → board y-down).

#![allow(dead_code)]

use std::collections::HashMap;
use std::path::Path;

use eframe::egui;
use crate::kicad9::{Element, FpArc, Footprint, Pad, PadShape};

/// Default colors used when drawing footprint elements (match main.rs).
pub const COPPER_COLOR: egui::Color32 = egui::Color32::from_rgb(0xb5, 0x73, 0x3c);
pub const MASK_COLOR: egui::Color32 = egui::Color32::from_rgb(0x18, 0x18, 0x18);
pub const OUTLINE_COLOR: egui::Color32 = egui::Color32::from_rgb(0x60, 0x60, 0x60);
/// KiCad-style silkscreen (white/cream).
pub const SILKSCREEN_COLOR: egui::Color32 = egui::Color32::from_rgb(0xff, 0xff, 0xe0);
/// Courtyard outline.
pub const COURTYARD_COLOR: egui::Color32 = egui::Color32::from_rgb(0x99, 0x33, 0xff);
/// Color for pad position crosshair (visible on copper).
const PAD_POSITION_COLOR: egui::Color32 = egui::Color32::from_rgb(0xff, 0xff, 0xc0);
/// Color for pad number labels.
const PAD_LABEL_COLOR: egui::Color32 = egui::Color32::from_rgb(0xe0, 0xe0, 0xe0);
/// Semi-transparent body fill under the footprint outline (dark gray, ~55% alpha).
fn body_fill_color() -> egui::Color32 {
    egui::Color32::from_rgba_unmultiplied(0x40, 0x40, 0x50, 140)
}

/// Layer names we consider for drawing (copper, mask, silk, fabrication, courtyard).
const DRAW_LAYERS: &[&str] = &[
    "F.Cu",
    "B.Cu",
    "F.Mask",
    "B.Mask",
    "F.SilkS",
    "B.SilkS",
    "F.Fab",   // Fabrication (body outline, lead lines) — used by Resistor_THT etc.
    "B.Fab",
    "F.CrtYd", // Courtyard
    "B.CrtYd",
];

/// KiCad pads often use wildcard layers "(layers \"*.Cu\" \"*.Mask\")"; treat those as drawable.
const PAD_LAYER_WILDCARDS: &[&str] = &["*.Cu", "*.Mask", "*.Paste"];

fn layer_drawable(layer: &str) -> bool {
    DRAW_LAYERS.iter().any(|&name| layer == name)
        || PAD_LAYER_WILDCARDS.iter().any(|&name| layer == name)
}

fn is_copper(layer: &str) -> bool {
    layer == "F.Cu" || layer == "B.Cu" || layer == "*.Cu"
}

fn is_silkscreen(layer: &str) -> bool {
    layer == "F.SilkS" || layer == "B.SilkS"
}

fn is_courtyard(layer: &str) -> bool {
    layer == "F.CrtYd" || layer == "B.CrtYd"
}

fn element_color(layer: &str) -> egui::Color32 {
    if is_copper(layer) {
        COPPER_COLOR
    } else if is_silkscreen(layer) {
        SILKSCREEN_COLOR
    } else if is_courtyard(layer) {
        COURTYARD_COLOR
    } else {
        OUTLINE_COLOR
    }
}

/// Load a single footprint from a `.kicad_mod` file using the in-tree kicad9 parser.
pub fn load_footprint(path: &Path) -> Result<Footprint, crate::kicad9::Error> {
    crate::kicad9::read_footprint(path)
}

/// Load all `.kicad_mod` files in a directory into a map keyed by footprint name.
/// Skips files that fail to parse so one bad file doesn't block the rest.
pub fn load_footprint_dir(dir: &Path) -> Result<HashMap<String, Footprint>, crate::kicad9::Error> {
    let mut map = HashMap::new();
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(std::ffi::OsStr::to_str) == Some("kicad_mod") {
            if let Ok(fp) = crate::kicad9::read_footprint(&path) {
                map.insert(fp.name.clone(), fp);
            }
        }
    }
    Ok(map)
}

/// Transform a point from footprint-local coordinates (KiCad: mm, y-up) to board position (cx, cy)
/// with optional rotation. Returns (board_x_mm, board_y_mm) with y flipped for screen.
#[inline]
fn transform_point(
    x: f64,
    y: f64,
    cx: f32,
    cy: f32,
    rot_deg: f64,
) -> (f32, f32) {
    let rad = rot_deg.to_radians();
    let cos = rad.cos() as f32;
    let sin = rad.sin() as f32;
    let xf = x as f32;
    let yf = y as f32;
    let rx = xf * cos - yf * sin;
    let ry = xf * sin + yf * cos;
    (cx + rx, cy - ry)
}

/// Check if (x, y) is in view (board mm).
#[inline]
fn in_view(x: f32, y: f32, view: (f32, f32, f32, f32)) -> bool {
    x >= view.0 && x <= view.1 && y >= view.2 && y <= view.3
}

/// Check if segment is in view.
#[inline]
fn segment_in_view(
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
    view: (f32, f32, f32, f32),
) -> bool {
    let (sx0, sx1) = (x0.min(x1), x0.max(x1));
    let (sy0, sy1) = (y0.min(y1), y0.max(y1));
    sx1 >= view.0 && sx0 <= view.1 && sy1 >= view.2 && sy0 <= view.3
}

/// Half-length of pad position cross in mm.
const PAD_POSITION_CROSS_MM: f32 = 0.2;

/// Draw a small cross at the pad center to show exact position.
fn draw_pad_position_marker(
    painter: &egui::Painter,
    to_screen: &impl Fn(f32, f32) -> egui::Pos2,
    scale: f32,
    bx: f32,
    by: f32,
) {
    let p = to_screen(bx, by);
    let h = (PAD_POSITION_CROSS_MM * scale).max(1.0);
    let stroke_w = (0.3 * scale).max(0.5);
    let stroke = egui::Stroke::new(stroke_w, PAD_POSITION_COLOR);
    painter.line_segment([p - egui::vec2(h, 0.0), p + egui::vec2(h, 0.0)], stroke);
    painter.line_segment([p - egui::vec2(0.0, h), p + egui::vec2(0.0, h)], stroke);
}

/// Draw a single pad (circle, rect, oval, trapezoid approximated).
fn draw_pad(
    painter: &egui::Painter,
    to_screen: &impl Fn(f32, f32) -> egui::Pos2,
    scale: f32,
    view: (f32, f32, f32, f32),
    pad: &Pad,
    cx: f32,
    cy: f32,
    rot_deg: f64,
) {
    let (px, py) = (pad.at.x, pad.at.y);
    let (bx, by) = transform_point(px, py, cx, cy, rot_deg);
    if !in_view(bx, by, view) {
        return;
    }
    let (sx, sy) = (pad.size.x as f32, pad.size.y as f32);
    let copper = is_copper_from_pad_layers(&pad.layers);
    let fill_color = if copper { COPPER_COLOR } else { OUTLINE_COLOR };

    match pad.shape {
        PadShape::Circle => {
            let r = (sx / 2.0).max(0.01);
            let p = to_screen(bx, by);
            painter.circle_filled(p, r * scale, fill_color);
            if let Some(ref drill) = pad.drill {
                let drill_r = drill_diameter(drill) / 2.0;
                if drill_r > 0.0 {
                    painter.circle_filled(p, drill_r * scale, MASK_COLOR);
                }
            }
        }
        PadShape::Rect | PadShape::Trapezoid => {
            let half_w = sx / 2.0;
            let half_h = sy / 2.0;
            let corners = [
                (-half_w, -half_h),
                (half_w, -half_h),
                (half_w, half_h),
                (-half_w, half_h),
            ];
            let pts: Vec<egui::Pos2> = corners
                .iter()
                .map(|&(dx, dy)| {
                    let (tx, ty) = transform_point(
                        px + dx as f64,
                        py + dy as f64,
                        cx,
                        cy,
                        rot_deg,
                    );
                    to_screen(tx, ty)
                })
                .collect();
            if pts.len() >= 3 {
                painter.add(egui::Shape::convex_polygon(
                    pts,
                    fill_color,
                    egui::Stroke::NONE,
                ));
            }
            if let Some(ref drill) = pad.drill {
                let drill_r = drill_diameter(drill) / 2.0;
                if drill_r > 0.0 {
                    let p = to_screen(bx, by);
                    painter.circle_filled(p, drill_r * scale, MASK_COLOR);
                }
            }
        }
        PadShape::Oval | PadShape::RoundRect => {
            let (rx, ry) = (sx / 2.0, sy / 2.0);
            let r = rx.max(ry).max(0.01);
            let p = to_screen(bx, by);
            painter.circle_filled(p, r * scale, fill_color);
            if let Some(ref drill) = pad.drill {
                let drill_r = drill_diameter(drill) / 2.0;
                if drill_r > 0.0 {
                    painter.circle_filled(p, drill_r * scale, MASK_COLOR);
                }
            }
        }
        PadShape::Custom => {
            // Fallback: draw as circle using size
            let r = (sx / 2.0).max(sy / 2.0).max(0.01);
            let p = to_screen(bx, by);
            painter.circle_filled(p, r * scale, fill_color);
            if let Some(ref drill) = pad.drill {
                let drill_r = drill_diameter(drill) / 2.0;
                if drill_r > 0.0 {
                    painter.circle_filled(p, drill_r * scale, MASK_COLOR);
                }
            }
        }
    }
    //draw_pad_position_marker(painter, to_screen, scale, bx, by);
}

fn is_copper_from_pad_layers(layers: &[String]) -> bool {
    layers.iter().any(|s| is_copper(s.as_str()))
}

fn drill_diameter(drill: &crate::kicad9::Drill) -> f32 {
    (drill.width as f32).max(drill.height as f32)
}

/// Margin (mm) added to footprint bounds for the body fill polygon.
const BODY_BOUNDS_MARGIN_MM: f32 = 0.5;

/// Transform a point from board/world coordinates to footprint-local (inverse of transform_point).
#[inline]
fn transform_point_inverse(
    world_x: f32,
    world_y: f32,
    cx: f32,
    cy: f32,
    rot_deg: f64,
) -> (f32, f32) {
    let dx = world_x - cx;
    let dy = world_y - cy;
    let rad = rot_deg.to_radians();
    let cos = rad.cos() as f32;
    let sin = rad.sin() as f32;
    let lx = dx * cos - dy * sin;
    let ly = -dx * sin - dy * cos;
    (lx, ly)
}

/// Returns true if (world_x, world_y) is inside the footprint's bounding box when the footprint
/// is at (cx, cy) with rotation rot_deg. Use for grab hit-testing.
pub fn point_in_footprint_bounds(
    world_x: f32,
    world_y: f32,
    cx: f32,
    cy: f32,
    rot_deg: f64,
    local_bounds: Option<(f32, f32, f32, f32)>,
) -> bool {
    let (lox, loy, hix, hiy) = match local_bounds {
        Some(b) => b,
        None => return false,
    };
    let (lx, ly) = transform_point_inverse(world_x, world_y, cx, cy, rot_deg);
    lx >= lox && lx <= hix && ly >= loy && ly <= hiy
}

/// Compute axis-aligned bounding box of the footprint in local coordinates (min_x, min_y, max_x, max_y).
/// Used to draw a semi-transparent body fill under the outline.
pub fn footprint_bounds_local(fp: &Footprint) -> Option<(f32, f32, f32, f32)> {
    let mut min_x = f32::MAX;
    let mut min_y = f32::MAX;
    let mut max_x = f32::MIN;
    let mut max_y = f32::MIN;

    macro_rules! extend {
        ($x:expr, $y:expr) => {
            let x = $x as f32;
            let y = $y as f32;
            min_x = min_x.min(x);
            min_y = min_y.min(y);
            max_x = max_x.max(x);
            max_y = max_y.max(y);
        };
    }

    for element in &fp.elements {
        match element {
            Element::Pad(pad) => {
                let hx = (pad.size.x as f32 / 2.0).max(0.5);
                let hy = (pad.size.y as f32 / 2.0).max(0.5);
                extend!(pad.at.x - hx as f64, pad.at.y - hy as f64);
                extend!(pad.at.x + hx as f64, pad.at.y + hy as f64);
            }
            Element::FpLine(line) => {
                extend!(line.start.x, line.start.y);
                extend!(line.end.x, line.end.y);
            }
            Element::FpRect(r) => {
                extend!(r.start.x, r.start.y);
                extend!(r.end.x, r.end.y);
            }
            Element::FpCircle(c) => {
                let radius = ((c.end.x - c.center.x).powi(2) + (c.end.y - c.center.y).powi(2)).sqrt();
                extend!(c.center.x - radius, c.center.y - radius);
                extend!(c.center.x + radius, c.center.y + radius);
            }
            Element::FpArc(arc) => {
                extend!(arc.start.x, arc.start.y);
                extend!(arc.mid.x, arc.mid.y);
                extend!(arc.end.x, arc.end.y);
                if let Some(((cx, cy), r, _)) = arc_center_radius_sweep_deg(arc) {
                    extend!(cx - r, cy - r);
                    extend!(cx + r, cy + r);
                }
            }
            Element::FpPoly(poly) => {
                for pt in &poly.pts {
                    extend!(pt.x, pt.y);
                }
            }
            _ => {}
        }
    }

    if min_x > max_x {
        return None;
    }
    let margin = BODY_BOUNDS_MARGIN_MM;
    Some((min_x - margin, min_y - margin, max_x + margin, max_y + margin))
}

/// Draw footprint graphic elements (lines, circles, arcs, polygons) and pads.
/// Footprint is drawn at (cx, cy) with rotation rot_deg (degrees, counterclockwise).
pub fn draw_footprint(
    painter: &egui::Painter,
    to_screen: &impl Fn(f32, f32) -> egui::Pos2,
    scale: f32,
    view: (f32, f32, f32, f32),
    fp: &Footprint,
    cx: f32,
    cy: f32,
    rot_deg: f64,
) {
    let rot = rot_deg;

    // Draw semi-transparent body fill under the outline so the footprint appears as a solid object.
    if let Some((lox, loy, hix, hiy)) = footprint_bounds_local(fp) {
        let corners = [
            (lox, loy),
            (hix, loy),
            (hix, hiy),
            (lox, hiy),
        ];
        let pts: Vec<egui::Pos2> = corners
            .iter()
            .map(|&(x, y)| {
                let (bx, by) = transform_point(x as f64, y as f64, cx, cy, rot);
                to_screen(bx, by)
            })
            .collect();
        if pts.len() >= 3 {
            painter.add(egui::Shape::convex_polygon(
                pts,
                body_fill_color(),
                egui::Stroke::NONE,
            ));
        }
    }

    for element in &fp.elements {
        match element {
            Element::Pad(pad) => {
                if pad.layers.iter().any(|s| layer_drawable(s.as_str())) {
                    draw_pad(painter, to_screen, scale, view, pad, cx, cy, rot);
                }
            }
            Element::FpLine(line) => {
                if layer_drawable(line.layer.as_str()) {
                    let (bx0, by0) = transform_point(line.start.x, line.start.y, cx, cy, rot);
                    let (bx1, by1) = transform_point(line.end.x, line.end.y, cx, cy, rot);
                    if !segment_in_view(bx0, by0, bx1, by1, view) {
                        continue;
                    }
                    let stroke_w = (line.width as f32 * scale).max(0.5);
                    let color = element_color(line.layer.as_str());
                    painter.line_segment(
                        [to_screen(bx0, by0), to_screen(bx1, by1)],
                        egui::Stroke::new(stroke_w, color),
                    );
                }
            }
            Element::FpCircle(circle) => {
                if layer_drawable(&circle.layer) {
                    let radius = ((circle.end.x - circle.center.x).powi(2)
                        + (circle.end.y - circle.center.y).powi(2))
                    .sqrt() as f32;
                    let (bx, by) = transform_point(
                        circle.center.x,
                        circle.center.y,
                        cx,
                        cy,
                        rot,
                    );
                    if !in_view(bx, by, view) {
                        continue;
                    }
                    let color = element_color(circle.layer.as_str());
                    let p = to_screen(bx, by);
                    let r = radius * scale;
                    if circle.width as f32 * scale >= r * 1.9 {
                        painter.circle_filled(p, r, color);
                    } else {
                        painter.circle_stroke(
                            p,
                            r,
                            egui::Stroke::new((circle.width as f32 * scale).max(0.5), color),
                        );
                    }
                }
            }
            Element::FpRect(rect) => {
                if layer_drawable(rect.layer.as_str()) {
                    let (sx, sy) = (rect.start.x, rect.start.y);
                    let (ex, ey) = (rect.end.x, rect.end.y);
                    let corners = [
                        (sx, sy),
                        (ex, sy),
                        (ex, ey),
                        (sx, ey),
                    ];
                    let pts: Vec<egui::Pos2> = corners
                        .iter()
                        .map(|&(x, y)| {
                            let (bx, by) = transform_point(x, y, cx, cy, rot);
                            to_screen(bx, by)
                        })
                        .collect();
                    let color = element_color(rect.layer.as_str());
                    let stroke_w = (rect.width as f32 * scale).max(0.5);
                    if rect.fill && pts.len() >= 3 {
                        painter.add(egui::Shape::convex_polygon(
                            pts.clone(),
                            color,
                            egui::Stroke::NONE,
                        ));
                    }
                    if !rect.fill || stroke_w > 0.5 {
                        for i in 0..pts.len() {
                            let a = pts[i];
                            let b = pts[(i + 1) % pts.len()];
                            painter.line_segment(
                                [a, b],
                                egui::Stroke::new(stroke_w, color),
                            );
                        }
                    }
                }
            }
            Element::FpArc(arc) => {
                if layer_drawable(arc.layer.as_str()) {
                    draw_arc(painter, to_screen, scale, view, arc, cx, cy, rot);
                }
            }
            Element::FpPoly(poly) => {
                if layer_drawable(poly.layer.as_str()) {
                    let pts: Vec<egui::Pos2> = poly
                        .pts
                        .iter()
                        .map(|xy| {
                            let (bx, by) =
                                transform_point(xy.x, xy.y, cx, cy, rot);
                            to_screen(bx, by)
                        })
                        .collect();
                    if pts.len() >= 3 {
                        let color = element_color(poly.layer.as_str());
                        painter.add(egui::Shape::convex_polygon(
                            pts,
                            color,
                            egui::Stroke::NONE,
                        ));
                    }
                }
            }
            _ => {}
        }
    }
    // Draw pad position labels (pad numbers) on top.
    let font_size = (7.0 * scale).max(5.0).min(12.0);
    let font_id = egui::FontId {
        size: font_size,
        family: egui::FontFamily::Name("Rockwell".into()),
    };
    for element in &fp.elements {
        if let Element::Pad(pad) = element {
            if !pad.layers.iter().any(|s| layer_drawable(s.as_str())) {
                continue;
            }
            let (px, py) = (pad.at.x, pad.at.y);
            let (bx, by) = transform_point(px, py, cx, cy, rot);
            if !in_view(bx, by, view) {
                continue;
            }
            let pos = to_screen(bx, by);
            painter.text(
                pos,
                egui::Align2::CENTER_CENTER,
                pad.name.as_str(),
                font_id.clone(),
                PAD_LABEL_COLOR,
            );
        }
    }
}


/// Compute arc center, radius and sweep angle from start, mid, end (KiCad 9 fp_arc).
fn arc_center_radius_sweep_deg(arc: &FpArc) -> Option<((f64, f64), f64, f64)> {
    let (sx, sy) = (arc.start.x, arc.start.y);
    let (mx, my) = (arc.mid.x, arc.mid.y);
    let (ex, ey) = (arc.end.x, arc.end.y);
    // Circumcircle of triangle (start, mid, end)
    let d = 2.0 * (sx * (my - ey) + mx * (ey - sy) + ex * (sy - my));
    if d.abs() < 1e-20 {
        return None;
    }
    let ux = ((sx * sx + sy * sy) * (my - ey)
        + (mx * mx + my * my) * (ey - sy)
        + (ex * ex + ey * ey) * (sy - my))
        / d;
    let uy = ((sx * sx + sy * sy) * (ex - mx)
        + (mx * mx + my * my) * (sx - ex)
        + (ex * ex + ey * ey) * (mx - sx))
        / d;
    let radius = ((sx - ux).powi(2) + (sy - uy).powi(2)).sqrt();
    if radius < 1e-10 {
        return None;
    }
    let start_angle = (sy - uy).atan2(sx - ux);
    let mid_angle = (my - uy).atan2(mx - ux);
    let end_angle = (ey - uy).atan2(ex - ux);
    // Sweep from start to end that passes through mid (choose the arc that contains mid)
    let mut sweep_rad = end_angle - start_angle;
    if sweep_rad > std::f64::consts::PI {
        sweep_rad -= 2.0 * std::f64::consts::PI;
    } else if sweep_rad < -std::f64::consts::PI {
        sweep_rad += 2.0 * std::f64::consts::PI;
    }
    let mid_in_sweep = {
        let mut a = mid_angle - start_angle;
        if a > std::f64::consts::PI {
            a -= 2.0 * std::f64::consts::PI;
        } else if a < -std::f64::consts::PI {
            a += 2.0 * std::f64::consts::PI;
        }
        (a >= 0.0) == (sweep_rad >= 0.0) && a.abs() <= sweep_rad.abs()
    };
    if !mid_in_sweep {
        sweep_rad = if sweep_rad >= 0.0 {
            sweep_rad - 2.0 * std::f64::consts::PI
        } else {
            sweep_rad + 2.0 * std::f64::consts::PI
        };
    }
    let sweep_deg = sweep_rad.to_degrees();
    Some(((ux, uy), radius, sweep_deg))
}

/// Approximate an arc with line segments (KiCad 9: start, mid, end).
fn draw_arc(
    painter: &egui::Painter,
    to_screen: &impl Fn(f32, f32) -> egui::Pos2,
    scale: f32,
    view: (f32, f32, f32, f32),
    arc: &FpArc,
    cx: f32,
    cy: f32,
    rot_deg: f64,
) {
    let ((cx_arc, cy_arc), radius, angle_deg) = match arc_center_radius_sweep_deg(arc) {
        Some(t) => t,
        None => return,
    };
    let n = (angle_deg.abs() / 10.0).ceil().max(2.0) as i32;
    let start_angle = (arc.start.y - cy_arc).atan2(arc.start.x - cx_arc);
    let mut prev = (
        cx_arc + radius * start_angle.cos(),
        cy_arc + radius * start_angle.sin(),
    );
    let color = element_color(arc.layer.as_str());
    let stroke_w = (arc.width as f32 * scale).max(0.5);
    let step = angle_deg.to_radians() / (n as f64);
    for i in 1..=n {
        let a = start_angle + step * (i as f64);
        let next = (
            cx_arc + radius * a.cos(),
            cy_arc + radius * a.sin(),
        );
        let (bx0, by0) = transform_point(prev.0, prev.1, cx, cy, rot_deg);
        let (bx1, by1) = transform_point(next.0, next.1, cx, cy, rot_deg);
        if segment_in_view(bx0, by0, bx1, by1, view) {
            painter.line_segment(
                [to_screen(bx0, by0), to_screen(bx1, by1)],
                egui::Stroke::new(stroke_w, color),
            );
        }
        prev = next;
    }
}
