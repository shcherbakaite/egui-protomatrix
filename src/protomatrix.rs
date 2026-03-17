//! Configurable protomatrix PCB component with event handling.
//!
//! Provides hit-testing and events for mouse over and mouse clicks on pads,
//! matrix rows, and solder jumpers.

use eframe::egui;
use serde::{Deserialize, Serialize};

/// Which side of the board (upper or lower proto area).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, PartialOrd, Ord)]
pub enum ProtoSide {
    Lower,
    Upper,
}

/// Target element under the cursor or clicked.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProtomatrixTarget {
    /// Proto pad at (col, row), 0-based.
    Pad { side: ProtoSide, col: i32, row: i32 },
    /// Matrix row index (0..matrix_size).
    MatrixRow { side: ProtoSide, row: i32 },
    /// Solder jumper at (col, row) in the matrix.
    SolderJumper { side: ProtoSide, col: i32, row: i32 },
}

/// Configuration for the protomatrix layout (matching protomatrix.py).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProtomatrixConfig {
    pub proto_area: (i32, i32),
    pub matrix_size: i32,
    pub matrix_break_every: i32,
    pub matrix_break_size_mm: f32,
    pub proto_pitch_mm: f32,
    pub matrix_v_pitch_mm: f32,
    pub proto_gap_mm: f32,
    pub track_width_mm: f32,
    pub jumper_track_offset_x_mm: f32,
    pub offset_100mil_mm: f32,
    pub offset_50mil_mm: f32,
    pub offset_30mil_mm: f32,
    pub proto_pad_dia_mm: f32,
    pub proto_pad_hole_mm: f32,
    pub via_drill_mm: f32,
    pub via_dia_mm: f32,
    pub jumper_pad_mm: f32,
    pub mounting_hole_mm: f32,
}

impl Default for ProtomatrixConfig {
    fn default() -> Self {
        Self {
            proto_area: (63, 5),
            matrix_size: 15,
            matrix_break_every: 10,
            matrix_break_size_mm: 0.508,
            proto_pitch_mm: 2.54,
            matrix_v_pitch_mm: 1.016,
            proto_gap_mm: 7.62,
            track_width_mm: 0.5,
            jumper_track_offset_x_mm: 0.381,
            offset_100mil_mm: 2.54,
            offset_50mil_mm: 1.27,
            offset_30mil_mm: 0.762,
            proto_pad_dia_mm: 1.9,
            proto_pad_hole_mm: 1.2,
            via_drill_mm: 0.3,
            via_dia_mm: 0.5,
            jumper_pad_mm: 0.635,
            mounting_hole_mm: 3.2,
        }
    }
}

impl ProtomatrixConfig {
    fn matrix_gradient(&self, _i: i32, j: i32) -> (f32, f32) {
        let mut dy = self.matrix_v_pitch_mm;
        if (j % self.matrix_break_every) == 0 && j != 0 {
            dy += self.matrix_break_size_mm;
        }
        (self.proto_pitch_mm, dy)
    }

    fn lower_matrix_offset(&self) -> (f32, f32) {
        (0.0, self.proto_area.1 as f32 * self.proto_pitch_mm)
    }

    fn upper_matrix_offset(&self) -> (f32, f32) {
        (0.0, -self.proto_area.1 as f32 * self.proto_pitch_mm - self.proto_gap_mm)
    }

    fn lower_matrix_row_y(&self, j: i32) -> f32 {
        let (_ox, oy) = self.lower_matrix_offset();
        let mut y = oy;
        for k in 0..j {
            let (_, dy) = self.matrix_gradient(0, k);
            y += dy;
        }
        y
    }

    fn upper_matrix_row_y(&self, j: i32) -> f32 {
        let (_, oy) = self.upper_matrix_offset();
        let mut y = oy;
        for k in 0..j {
            let (_, dy) = self.matrix_gradient(0, k);
            y -= dy;
        }
        y
    }

    fn matrix_row_link_x(&self, j: i32) -> f32 {
        let half = self.matrix_size / 2;
        -self.offset_100mil_mm - (j % half) as f32 * self.offset_30mil_mm
    }

    fn matrix_row_end_x(&self) -> f32 {
        (self.proto_area.0 as f32 - 1.0) * self.proto_pitch_mm - self.offset_50mil_mm
    }
}

/// Solder jumper polygon vertices (right half, mm; footprint origin at center; KiCad y-up).
const SOLDER_JUMPER_RIGHT_HALF_MM: &[(f32, f32)] = &[
    (-0.05, -0.3175),
    (0.573_875, -0.3175),
    (0.621_567, -0.308_013),
    (0.661_998, -0.280_998),
    (0.689_013, -0.240_567),
    (0.698_499, -0.192_874),
    (0.698_5, 0.192_875),
    (0.689_013, 0.240_567),
    (0.661_998, 0.280_998),
    (0.621_567, 0.308_013),
    (0.573_874, 0.317_499),
    (0.325, 0.3175),
];
const SOLDER_JUMPER_LEFT_HALF_MM: &[(f32, f32)] = &[
    (0.05, 0.3175),
    (-0.555_625, 0.3175),
    (-0.610_301, 0.306_624),
    (-0.656_653, 0.275_653),
    (-0.687_624, 0.229_301),
    (-0.698_499, 0.174_624),
    (-0.698_5, -0.174_625),
    (-0.687_624, -0.229_301),
    (-0.656_653, -0.275_653),
    (-0.610_301, -0.306_624),
    (-0.555_624, -0.317_499),
    (-0.325, -0.3175),
];

fn point_in_polygon(px: f32, py: f32, cx: f32, cy: f32, verts: &[(f32, f32)]) -> bool {
    // verts are in footprint-local (fx, fy); we transform px,py to local: lx = px - cx, ly = cy - py (y flip)
    let lx = px - cx;
    let ly = cy - py; // KiCad y-up: board cy - py
    let n = verts.len();
    let mut inside = false;
    let mut j = n - 1;
    for i in 0..n {
        let (xi, yi) = verts[i];
        let (xj, yj) = verts[j];
        if ((yi > ly) != (yj > ly)) && (lx < (xj - xi) * (ly - yi) / (yj - yi) + xi) {
            inside = !inside;
        }
        j = i;
    }
    inside
}

fn point_in_solder_jumper(px: f32, py: f32, cx: f32, cy: f32) -> bool {
    point_in_polygon(px, py, cx, cy, SOLDER_JUMPER_RIGHT_HALF_MM)
        || point_in_polygon(px, py, cx, cy, SOLDER_JUMPER_LEFT_HALF_MM)
}

/// Hit-test priority: solder jumper (smallest) > pad > matrix row (largest area).
/// Returns the highest-priority target at the given point, or None.
pub fn hit_test(config: &ProtomatrixConfig, x_mm: f32, y_mm: f32) -> Option<ProtomatrixTarget> {
    // 1. Solder jumpers (both matrix areas)
    let (ox_l, _) = config.lower_matrix_offset();
    for j in 0..config.matrix_size {
        let row_y = config.lower_matrix_row_y(j);
        for i in 0..config.proto_area.0 {
            let sx = ox_l + i as f32 * config.proto_pitch_mm;
            if point_in_solder_jumper(x_mm, y_mm, sx, row_y) {
                return Some(ProtomatrixTarget::SolderJumper {
                    side: ProtoSide::Lower,
                    col: i,
                    row: j,
                });
            }
        }
    }
    let (ox_u, _) = config.upper_matrix_offset();
    for j in 0..config.matrix_size {
        let row_y = config.upper_matrix_row_y(j);
        for i in 0..config.proto_area.0 {
            let sx = ox_u + i as f32 * config.proto_pitch_mm;
            if point_in_solder_jumper(x_mm, y_mm, sx, row_y) {
                return Some(ProtomatrixTarget::SolderJumper {
                    side: ProtoSide::Upper,
                    col: i,
                    row: j,
                });
            }
        }
    }

    // 2. Proto pads (use pad outer radius for hit; hole is smaller)
    let pad_r = config.proto_pad_dia_mm / 2.0;
    for i in 0..config.proto_area.0 {
        for j in 0..config.proto_area.1 {
            let px = i as f32 * config.proto_pitch_mm;
            let py = j as f32 * config.proto_pitch_mm;
            let dsq = (x_mm - px).powi(2) + (y_mm - py).powi(2);
            if dsq <= pad_r * pad_r {
                return Some(ProtomatrixTarget::Pad {
                    side: ProtoSide::Lower,
                    col: i,
                    row: j,
                });
            }
        }
    }
    for i in 0..config.proto_area.0 {
        for j in 0..config.proto_area.1 {
            let px = i as f32 * config.proto_pitch_mm;
            let py = -config.proto_gap_mm - j as f32 * config.proto_pitch_mm;
            let dsq = (x_mm - px).powi(2) + (y_mm - py).powi(2);
            if dsq <= pad_r * pad_r {
                return Some(ProtomatrixTarget::Pad {
                    side: ProtoSide::Upper,
                    col: i,
                    row: j,
                });
            }
        }
    }

    // 3. Matrix rows (horizontal band)
    let half = config.matrix_size / 2;
    let row_hit_margin = config.matrix_v_pitch_mm * 0.6;
    let link_x_min = -config.offset_100mil_mm - (half - 1) as f32 * config.offset_30mil_mm;
    let end_x = config.matrix_row_end_x();

    for j in 0..config.matrix_size {
        let row_y = config.lower_matrix_row_y(j);
        let link_x = config.matrix_row_link_x(j);
        if y_mm >= row_y - row_hit_margin
            && y_mm <= row_y + row_hit_margin
            && x_mm >= link_x_min.min(link_x)
            && x_mm <= end_x
        {
            return Some(ProtomatrixTarget::MatrixRow {
                side: ProtoSide::Lower,
                row: j,
            });
        }
    }
    for j in 0..config.matrix_size {
        let row_y = config.upper_matrix_row_y(j);
        let link_x = config.matrix_row_link_x(j);
        if y_mm >= row_y - row_hit_margin
            && y_mm <= row_y + row_hit_margin
            && x_mm >= link_x_min.min(link_x)
            && x_mm <= end_x
        {
            return Some(ProtomatrixTarget::MatrixRow {
                side: ProtoSide::Upper,
                row: j,
            });
        }
    }

    None
}

/// Position in mm for a pad target; returns None for non-pad targets.
pub fn target_to_mm(config: &ProtomatrixConfig, target: &ProtomatrixTarget) -> Option<(f32, f32)> {
    match target {
        ProtomatrixTarget::Pad { side, col, row } => {
            let x = *col as f32 * config.proto_pitch_mm;
            let y = match side {
                ProtoSide::Lower => *row as f32 * config.proto_pitch_mm,
                ProtoSide::Upper => -config.proto_gap_mm - *row as f32 * config.proto_pitch_mm,
            };
            Some((x, y))
        }
        _ => None,
    }
}

/// Board bounds in mm (x_min, x_max, y_min, y_max) for layout.
pub fn board_bounds_mm(config: &ProtomatrixConfig) -> (f32, f32, f32, f32) {
    let x_max = (config.proto_area.0 as f32 - 1.0) * config.proto_pitch_mm + 5.0;
    let x_min = -config.offset_100mil_mm - 15.0 * config.offset_30mil_mm - 2.0;
    let y_lower_matrix_end = config.proto_area.1 as f32 * config.proto_pitch_mm
        + (config.matrix_size as f32 * config.matrix_v_pitch_mm)
        + (config.matrix_size / config.matrix_break_every) as f32 * config.matrix_break_size_mm;
    let y_upper_matrix_top = -config.proto_gap_mm
        - config.proto_area.1 as f32 * config.proto_pitch_mm
        - (config.matrix_size as f32 * config.matrix_v_pitch_mm)
        - (config.matrix_size / config.matrix_break_every) as f32 * config.matrix_break_size_mm;
    (
        x_min,
        x_max,
        y_upper_matrix_top - 2.0,
        y_lower_matrix_end + 2.0,
    )
}

// --- Drawing (reusable with config) ---

const COPPER_COLOR: egui::Color32 = egui::Color32::from_rgb(0xb5, 0x73, 0x3c);

/// Color palette for net defaults.
pub const NET_COLORS: [egui::Color32; 12] = [
    egui::Color32::from_rgb(0x40, 0xc0, 0x60), // green
    egui::Color32::from_rgb(0x60, 0x80, 0xff), // blue
    egui::Color32::from_rgb(0xff, 0x80, 0x40), // orange
    egui::Color32::from_rgb(0xc0, 0x40, 0xff), // purple
    egui::Color32::from_rgb(0xff, 0xc0, 0x40), // yellow
    egui::Color32::from_rgb(0x40, 0xff, 0xff), // cyan
    egui::Color32::from_rgb(0xff, 0x40, 0x80), // pink
    egui::Color32::from_rgb(0x80, 0xff, 0x40), // lime
    egui::Color32::from_rgb(0xff, 0x60, 0x40), // coral
    egui::Color32::from_rgb(0x60, 0x40, 0xff), // indigo
    egui::Color32::from_rgb(0xff, 0xff, 0x60), // light yellow
    egui::Color32::from_rgb(0x80, 0xc0, 0xff), // light blue
];

/// Default color for a net index (from vibrant palette).
pub fn net_color(net_index: usize) -> egui::Color32 {
    NET_COLORS[net_index % NET_COLORS.len()]
}

const MASK_COLOR: egui::Color32 = egui::Color32::from_rgb(0x18, 0x18, 0x18);
const OUTLINE_COLOR: egui::Color32 = egui::Color32::from_rgb(0x60, 0x60, 0x60);
const HOLE_COLOR: egui::Color32 = egui::Color32::from_rgb(0x28, 0x20, 0x1c);
/// KiCad-style silkscreen (white/cream).
const SILKSCREEN_COLOR: egui::Color32 = egui::Color32::from_rgb(0xf0, 0xf0, 0xe8);

/// Trait for querying which solder jumpers should be closed (autorouter output).
pub trait JumperStateProvider {
    fn is_closed(&self, side: ProtoSide, col: i32, row: i32) -> bool;
    /// Net index for closed jumpers (0-based); used for per-net coloring.
    fn net_index(&self, side: ProtoSide, col: i32, row: i32) -> Option<usize> {
        if self.is_closed(side, col, row) {
            Some(0)
        } else {
            None
        }
    }
}

fn track_color() -> egui::Color32 {
    egui::Color32::from_rgba_unmultiplied(128, 128, 128, 140)
}

#[inline(always)]
fn in_view(x: f32, y: f32, v: (f32, f32, f32, f32)) -> bool {
    x >= v.0 && x <= v.1 && y >= v.2 && y <= v.3
}

#[inline(always)]
fn segment_in_view(x0: f32, y0: f32, x1: f32, y1: f32, v: (f32, f32, f32, f32)) -> bool {
    let (sx0, sx1) = (x0.min(x1), x0.max(x1));
    let (sy0, sy1) = (y0.min(y1), y0.max(y1));
    sx1 >= v.0 && sx0 <= v.1 && sy1 >= v.2 && sy0 <= v.3
}

fn draw_solder_jumper_pad(
    painter: &egui::Painter,
    to_screen: &impl Fn(f32, f32) -> egui::Pos2,
    cx: f32,
    cy: f32,
    fill_color: egui::Color32,
) {
    let to_pt = |fx: f32, fy: f32| to_screen(cx + fx, cy - fy);
    let right_pts: Vec<egui::Pos2> = SOLDER_JUMPER_RIGHT_HALF_MM
        .iter()
        .map(|&(fx, fy)| to_pt(fx, fy))
        .collect();
    let left_pts: Vec<egui::Pos2> = SOLDER_JUMPER_LEFT_HALF_MM
        .iter()
        .map(|&(fx, fy)| to_pt(fx, fy))
        .collect();
    painter.add(egui::Shape::convex_polygon(
        right_pts,
        fill_color,
        egui::Stroke::NONE,
    ));
    painter.add(egui::Shape::convex_polygon(
        left_pts,
        fill_color,
        egui::Stroke::NONE,
    ));
}

/// Draw jumper indicators on proto pads that are connected via the matrix.
/// Shows a colored ring around each pad that belongs to a net (closed jumper in that column).
/// If `net_color_override` is Some, it maps net_index -> color; otherwise uses default palette.
pub fn draw_proto_jumper_indicators<J: JumperStateProvider + ?Sized>(
    config: &ProtomatrixConfig,
    painter: &egui::Painter,
    to_screen: &impl Fn(f32, f32) -> egui::Pos2,
    scale: f32,
    view: (f32, f32, f32, f32),
    jumper_state: Option<&J>,
    net_color_override: Option<&dyn Fn(usize) -> egui::Color32>,
) {
    let Some(js) = jumper_state else {
        return;
    };
    // Build (side, col) -> net_index from closed jumpers (JumperStateProvider has no iter, so we probe)
    let mut col_net: std::collections::HashMap<(ProtoSide, i32), usize> =
        std::collections::HashMap::new();
    for side in [ProtoSide::Lower, ProtoSide::Upper] {
        for col in 0..config.proto_area.0 {
            for matrix_row in 0..config.matrix_size {
                if let Some(net_idx) = js.net_index(side, col, matrix_row) {
                    col_net.insert((side, col), net_idx);
                    break; // one jumper per column per net
                }
            }
        }
    }
    let pad_r = config.proto_pad_dia_mm / 2.0;
    let stroke_w = (0.3_f32).max(config.track_width_mm * 0.6) * scale;
    let color_for = |net_idx: usize| {
        net_color_override
            .map(|f| f(net_idx))
            .unwrap_or_else(|| net_color(net_idx))
    };
    // Lower proto area
    for i in 0..config.proto_area.0 {
        if let Some(&net_idx) = col_net.get(&(ProtoSide::Lower, i)) {
            let color = color_for(net_idx);
            for j in 0..config.proto_area.1 {
                let x = i as f32 * config.proto_pitch_mm;
                let y = j as f32 * config.proto_pitch_mm;
                if !in_view(x, y, view) {
                    continue;
                }
                let c = to_screen(x, y);
                painter.circle_stroke(c, pad_r * scale, egui::Stroke::new(stroke_w, color));
            }
        }
    }
    // Upper proto area
    for i in 0..config.proto_area.0 {
        if let Some(&net_idx) = col_net.get(&(ProtoSide::Upper, i)) {
            let color = color_for(net_idx);
            for j in 0..config.proto_area.1 {
                let x = i as f32 * config.proto_pitch_mm;
                let y = -config.proto_gap_mm - j as f32 * config.proto_pitch_mm;
                if !in_view(x, y, view) {
                    continue;
                }
                let c = to_screen(x, y);
                painter.circle_stroke(c, pad_r * scale, egui::Stroke::new(stroke_w, color));
            }
        }
    }
}

/// Draw proto pads (both sides).
pub fn draw_proto_pads(
    config: &ProtomatrixConfig,
    painter: &egui::Painter,
    to_screen: &impl Fn(f32, f32) -> egui::Pos2,
    scale: f32,
    view: (f32, f32, f32, f32),
) {
    let r = config.proto_pad_dia_mm / 2.0;
    for i in 0..config.proto_area.0 {
        for j in 0..config.proto_area.1 {
            let x = i as f32 * config.proto_pitch_mm;
            let y = j as f32 * config.proto_pitch_mm;
            if !in_view(x, y, view) {
                continue;
            }
            let c = to_screen(x, y);
            painter.circle_filled(c, r * scale, COPPER_COLOR);
            painter.circle_filled(c, (config.proto_pad_hole_mm / 2.0) * scale, MASK_COLOR);
        }
    }
    for i in 0..config.proto_area.0 {
        for j in 0..config.proto_area.1 {
            let x = i as f32 * config.proto_pitch_mm;
            let y = -config.proto_gap_mm - j as f32 * config.proto_pitch_mm;
            if !in_view(x, y, view) {
                continue;
            }
            let c = to_screen(x, y);
            painter.circle_filled(c, r * scale, COPPER_COLOR);
            painter.circle_filled(c, (config.proto_pad_hole_mm / 2.0) * scale, MASK_COLOR);
        }
    }
}

/// Draw matrix areas (solder jumpers + vias).
/// If `jumper_state` is provided, closed jumpers are highlighted.
/// If `net_color_override` is Some, it maps net_index -> color; otherwise uses default palette.
pub fn draw_matrix_areas<J: JumperStateProvider + ?Sized>(
    config: &ProtomatrixConfig,
    painter: &egui::Painter,
    to_screen: &impl Fn(f32, f32) -> egui::Pos2,
    scale: f32,
    view: (f32, f32, f32, f32),
    jumper_state: Option<&J>,
    net_color_override: Option<&dyn Fn(usize) -> egui::Color32>,
) {
    let (ox, _) = config.lower_matrix_offset();
    for j in 0..config.matrix_size {
        let y = config.lower_matrix_row_y(j);
        for i in 0..config.proto_area.0 {
            let x = ox + i as f32 * config.proto_pitch_mm;
            if !in_view(x, y, view) {
                continue;
            }
            let fill_color = jumper_state
                .and_then(|js| js.net_index(ProtoSide::Lower, i, j))
                .map(|ni| {
                    net_color_override
                        .map(|f| f(ni))
                        .unwrap_or_else(|| net_color(ni))
                })
                .unwrap_or(COPPER_COLOR);
            draw_solder_jumper_pad(painter, to_screen, x, y, fill_color);
            let via_x = x - config.proto_pitch_mm / 2.0;
            let vp = to_screen(via_x, y);
            painter.circle_filled(vp, (config.via_dia_mm / 2.0) * scale, COPPER_COLOR);
            painter.circle_filled(vp, (config.via_drill_mm / 2.0) * scale, MASK_COLOR);
        }
    }
    let (ox, _) = config.upper_matrix_offset();
    for j in 0..config.matrix_size {
        let y = config.upper_matrix_row_y(j);
        for i in 0..config.proto_area.0 {
            let x = ox + i as f32 * config.proto_pitch_mm;
            if !in_view(x, y, view) {
                continue;
            }
            let fill_color = jumper_state
                .and_then(|js| js.net_index(ProtoSide::Upper, i, j))
                .map(|ni| {
                    net_color_override
                        .map(|f| f(ni))
                        .unwrap_or_else(|| net_color(ni))
                })
                .unwrap_or(COPPER_COLOR);
            draw_solder_jumper_pad(painter, to_screen, x, y, fill_color);
            let via_x = x - config.proto_pitch_mm / 2.0;
            let vp = to_screen(via_x, y);
            painter.circle_filled(vp, (config.via_dia_mm / 2.0) * scale, COPPER_COLOR);
            painter.circle_filled(vp, (config.via_drill_mm / 2.0) * scale, MASK_COLOR);
        }
    }
}

/// Draw tracks.
/// Uses `Shape::line` for multi-point paths so segments connect without gaps at joints.
pub fn draw_tracks(
    config: &ProtomatrixConfig,
    painter: &egui::Painter,
    to_screen: &impl Fn(f32, f32) -> egui::Pos2,
    scale: f32,
    view: (f32, f32, f32, f32),
) {
    let stroke_w = config.track_width_mm * scale;
    let stroke = egui::Stroke::new(stroke_w, track_color());

    // Vertical proto tracks: one continuous path per column (avoids gaps at pad junctions).
    for i in 0..config.proto_area.0 {
        let x = i as f32 * config.proto_pitch_mm;
        let pts: Vec<egui::Pos2> = (0..config.proto_area.1)
            .map(|j| to_screen(x, j as f32 * config.proto_pitch_mm))
            .collect();
        if pts.len() >= 2 && segment_in_view(x, 0.0, x, (config.proto_area.1 - 1) as f32 * config.proto_pitch_mm, view) {
            painter.add(egui::Shape::line(pts, stroke));
        }
    }
    for i in 0..config.proto_area.0 {
        let x = i as f32 * config.proto_pitch_mm;
        let pts: Vec<egui::Pos2> = (0..config.proto_area.1)
            .map(|j| to_screen(x, -config.proto_gap_mm - j as f32 * config.proto_pitch_mm))
            .collect();
        let y0 = -config.proto_gap_mm;
        let y1 = -config.proto_gap_mm - (config.proto_area.1 - 1) as f32 * config.proto_pitch_mm;
        if pts.len() >= 2 && segment_in_view(x, y0, x, y1, view) {
            painter.add(egui::Shape::line(pts, stroke));
        }
    }

    // Matrix row horizontal + vertical link: one path per row so corner at link_x connects.
    let center_y = -config.proto_gap_mm / 2.0;
    let end_x = config.matrix_row_end_x();
    for j in 0..config.matrix_size {
        let row_y = config.lower_matrix_row_y(j);
        let link_x = config.matrix_row_link_x(j);
        if !segment_in_view(link_x, center_y.max(row_y), end_x, center_y.min(row_y), view) {
            continue;
        }
        let pts = vec![
            to_screen(link_x, center_y),
            to_screen(link_x, row_y),
            to_screen(end_x, row_y),
        ];
        painter.add(egui::Shape::line(pts, stroke));
    }
    for j in 0..config.matrix_size {
        let row_y = config.upper_matrix_row_y(j);
        let link_x = config.matrix_row_link_x(j);
        if !segment_in_view(link_x, center_y.min(row_y), end_x, center_y.max(row_y), view) {
            continue;
        }
        let pts = vec![
            to_screen(link_x, center_y),
            to_screen(link_x, row_y),
            to_screen(end_x, row_y),
        ];
        painter.add(egui::Shape::line(pts, stroke));
    }

    // Vertical jumper tracks between rows (single segments).
    for j in 0..config.matrix_size - 1 {
        let y0 = config.lower_matrix_row_y(j);
        let y1 = config.lower_matrix_row_y(j + 1);
        for i in 0..config.proto_area.0 {
            let px = i as f32 * config.proto_pitch_mm + config.jumper_track_offset_x_mm;
            if !segment_in_view(px, y0, px, y1, view) {
                continue;
            }
            painter.line_segment([to_screen(px, y0), to_screen(px, y1)], stroke);
        }
    }
    for j in 0..config.matrix_size - 1 {
        let y0 = config.upper_matrix_row_y(j);
        let y1 = config.upper_matrix_row_y(j + 1);
        for i in 0..config.proto_area.0 {
            let px = i as f32 * config.proto_pitch_mm + config.jumper_track_offset_x_mm;
            if !segment_in_view(px, y0, px, y1, view) {
                continue;
            }
            painter.line_segment([to_screen(px, y0), to_screen(px, y1)], stroke);
        }
    }

    // Lower X linking: pad -> half -> knob -> end (one path per column).
    for i in 0..config.proto_area.0 {
        let x = i as f32 * config.proto_pitch_mm;
        let y_bottom = (config.proto_area.1 - 1) as f32 * config.proto_pitch_mm;
        let y_half = y_bottom + config.proto_pitch_mm / 2.0;
        let y_knob = y_half + 0.381;
        let cx = x + config.jumper_track_offset_x_mm;
        let y_end = config.proto_area.1 as f32 * config.proto_pitch_mm;
        if !segment_in_view(x.min(cx), y_bottom, x.max(cx), y_end, view) {
            continue;
        }
        let pts = vec![
            to_screen(x, y_bottom),
            to_screen(x, y_half),
            to_screen(cx, y_knob),
            to_screen(cx, y_end),
        ];
        painter.add(egui::Shape::line(pts, stroke));
    }
    // Upper X linking: same pattern.
    let y_upper_bottom_pad = -config.proto_gap_mm
        - (config.proto_area.1 - 1) as f32 * config.proto_pitch_mm;
    let y_upper_half = y_upper_bottom_pad - config.proto_pitch_mm / 2.0;
    let y_upper_knob = y_upper_half - 0.381;
    let y_upper_end = -config.proto_gap_mm
        - config.proto_area.1 as f32 * config.proto_pitch_mm;
    for i in 0..config.proto_area.0 {
        let x = i as f32 * config.proto_pitch_mm;
        let cx = x + config.jumper_track_offset_x_mm;
        if !segment_in_view(x.min(cx), y_upper_end, x.max(cx), y_upper_bottom_pad, view) {
            continue;
        }
        let pts = vec![
            to_screen(x, y_upper_bottom_pad),
            to_screen(x, y_upper_half),
            to_screen(cx, y_upper_knob),
            to_screen(cx, y_upper_end),
        ];
        painter.add(egui::Shape::line(pts, stroke));
    }
}

/// Draw Y link vias.
pub fn draw_y_link_vias(
    config: &ProtomatrixConfig,
    painter: &egui::Painter,
    to_screen: &impl Fn(f32, f32) -> egui::Pos2,
    scale: f32,
    view: (f32, f32, f32, f32),
) {
    let half = config.matrix_size / 2;
    for j in half..config.matrix_size {
        let row_y = config.lower_matrix_row_y(j);
        let link_x = config.matrix_row_link_x(j);
        if !in_view(link_x, row_y, view) {
            continue;
        }
        let vp = to_screen(link_x, row_y);
        painter.circle_filled(vp, (config.via_dia_mm / 2.0) * scale, COPPER_COLOR);
        painter.circle_filled(vp, (config.via_drill_mm / 2.0) * scale, MASK_COLOR);
    }
    for j in half..config.matrix_size {
        let row_y = config.upper_matrix_row_y(j);
        let link_x = config.matrix_row_link_x(j);
        if !in_view(link_x, row_y, view) {
            continue;
        }
        let vp = to_screen(link_x, row_y);
        painter.circle_filled(vp, (config.via_dia_mm / 2.0) * scale, COPPER_COLOR);
        painter.circle_filled(vp, (config.via_drill_mm / 2.0) * scale, MASK_COLOR);
    }
}

/// Draw mounting holes.
pub fn draw_mounting_holes(
    config: &ProtomatrixConfig,
    painter: &egui::Painter,
    to_screen: &impl Fn(f32, f32) -> egui::Pos2,
    scale: f32,
    view: (f32, f32, f32, f32),
) {
    let center_y = -config.proto_gap_mm / 2.0;
    let r = (config.mounting_hole_mm / 2.0) * scale;
    let positions = [
        config.proto_pitch_mm / 2.0,
        (config.proto_area.0 as f32 - 1.0) * config.proto_pitch_mm / 2.0,
        (config.proto_area.0 as f32 - 1.0) * config.proto_pitch_mm - config.proto_pitch_mm / 2.0,
    ];
    for &hx in &positions {
        if !in_view(hx, center_y, view) {
            continue;
        }
        let c = to_screen(hx, center_y);
        painter.circle_filled(c, r, HOLE_COLOR);
        painter.circle_stroke(c, r, egui::Stroke::new(1.0, OUTLINE_COLOR));
    }
}

/// Proto column labels (letters a,b,c,... for proto rows, matching protomatrix.py BuildProtoColumnLabels).
/// Lower proto rows get letters starting at index proto_area[1]; upper rows get a..e (reversed).
pub fn draw_silkscreen_proto_column_labels(
    config: &ProtomatrixConfig,
    painter: &egui::Painter,
    to_screen: &impl Fn(f32, f32) -> egui::Pos2,
    scale: f32,
    view: (f32, f32, f32, f32),
) {
    const LETTERS: &str = "abcdefghijklmnopqrstuvxyz";
    let label_x = -config.offset_100mil_mm;
    let font_size = (1.016_f32 * scale).max(6.0).min(14.0); // ~40mil base
    let font_id = egui::FontId::proportional(font_size);

    // Lower proto: one label per row, letters f,g,h,... (index j + proto_area[1])
    for j in 0..config.proto_area.1 {
        let y = j as f32 * config.proto_pitch_mm;
        if !in_view(label_x, y, view) {
            continue;
        }
        let idx = (j + config.proto_area.1) as usize;
        let ch = LETTERS.chars().nth(idx).unwrap_or('?');
        let pos = to_screen(label_x, y);
        painter.text(
            pos,
            egui::Align2::RIGHT_CENTER,
            ch.to_string(),
            font_id.clone(),
            SILKSCREEN_COLOR,
        );
    }

    // Upper proto: one label per row, letters e,d,c,b,a (index proto_area[1]-j-1)
    for j in 0..config.proto_area.1 {
        let y = -config.proto_gap_mm - j as f32 * config.proto_pitch_mm;
        if !in_view(label_x, y, view) {
            continue;
        }
        let idx = (config.proto_area.1 - j - 1) as usize;
        let ch = LETTERS.chars().nth(idx).unwrap_or('?');
        let pos = to_screen(label_x, y);
        painter.text(
            pos,
            egui::Align2::RIGHT_CENTER,
            ch.to_string(),
            font_id.clone(),
            SILKSCREEN_COLOR,
        );
    }
}

/// X column labels (X1a, X2a, ... for upper; X1b, X2b, ... for lower, matching protomatrix.py BuildMatrixXLabels).
/// Placed at the last row of each matrix, offset 100mil above/below.
pub fn draw_silkscreen_x_labels(
    config: &ProtomatrixConfig,
    painter: &egui::Painter,
    to_screen: &impl Fn(f32, f32) -> egui::Pos2,
    scale: f32,
    view: (f32, f32, f32, f32),
) {
    let offset_100mil = config.offset_100mil_mm;
    let last_row = config.matrix_size - 1;

    // Lower matrix: X1b, X2b, ... below last row
    let row_y = config.lower_matrix_row_y(last_row);
    let label_y = row_y + offset_100mil;
    for i in 0..config.proto_area.0 {
        let x = i as f32 * config.proto_pitch_mm;
        if !in_view(x, label_y, view) {
            continue;
        }
        let font_size = if (i + 1) % 10 == 0 {
            (0.889_f32 * scale).max(6.0).min(14.0) // ~35mil
        } else {
            (0.635_f32 * scale).max(5.0).min(11.0) // ~25mil
        };
        let font_id = egui::FontId::proportional(font_size);
        let pos = to_screen(x, label_y);
        painter.text(
            pos,
            egui::Align2::CENTER_TOP,
            format!("X{}b", i + 1),
            font_id,
            SILKSCREEN_COLOR,
        );
    }

    // Upper matrix: X1a, X2a, ... above last row
    let row_y = config.upper_matrix_row_y(last_row);
    let label_y = row_y - offset_100mil;
    for i in 0..config.proto_area.0 {
        let x = i as f32 * config.proto_pitch_mm;
        if !in_view(x, label_y, view) {
            continue;
        }
        let font_size = if (i + 1) % 10 == 0 {
            (0.889_f32 * scale).max(6.0).min(14.0) // ~35mil
        } else {
            (0.635_f32 * scale).max(5.0).min(11.0) // ~25mil
        };
        let font_id = egui::FontId::proportional(font_size);
        let pos = to_screen(x, label_y);
        painter.text(
            pos,
            egui::Align2::CENTER_BOTTOM,
            format!("X{}a", i + 1),
            font_id,
            SILKSCREEN_COLOR,
        );
    }
}

/// Y row labels (Y1, Y2, ... for matrix rows, matching protomatrix.py BuildMatrixYLabels).
pub fn draw_silkscreen_y_labels(
    config: &ProtomatrixConfig,
    painter: &egui::Painter,
    to_screen: &impl Fn(f32, f32) -> egui::Pos2,
    scale: f32,
    view: (f32, f32, f32, f32),
) {
    let label_x = -config.offset_100mil_mm;
    let font_size = (0.635_f32 * scale).max(5.0).min(11.0); // ~25mil base
    let font_id = egui::FontId::proportional(font_size);

    for j in 0..config.matrix_size {
        let row_y = config.lower_matrix_row_y(j);
        if !in_view(label_x, row_y, view) {
            continue;
        }
        let pos = to_screen(label_x, row_y);
        painter.text(
            pos,
            egui::Align2::RIGHT_CENTER,
            format!("Y{}", j + 1),
            font_id.clone(),
            SILKSCREEN_COLOR,
        );
    }
    for j in 0..config.matrix_size {
        let row_y = config.upper_matrix_row_y(j);
        if !in_view(label_x, row_y, view) {
            continue;
        }
        let pos = to_screen(label_x, row_y);
        painter.text(
            pos,
            egui::Align2::RIGHT_CENTER,
            format!("Y{}", j + 1),
            font_id.clone(),
            SILKSCREEN_COLOR,
        );
    }
}

/// Grey color for the connection drag preview line.
const DRAG_LINE_COLOR: egui::Color32 = egui::Color32::from_rgb(0x80, 0x80, 0x80);

/// Draw a grey line from source (pad) to current pointer position while dragging to create a connection.
/// Call only when actively dragging; line is hidden when released.
pub fn draw_connection_drag_line(
    config: &ProtomatrixConfig,
    painter: &egui::Painter,
    to_screen: &impl Fn(f32, f32) -> egui::Pos2,
    scale: f32,
    source: &ProtomatrixTarget,
    pointer_mm: egui::Vec2,
) {
    let Some((sx, sy)) = target_to_mm(config, source) else {
        return;
    };
    let stroke_w = config.track_width_mm * scale * 1.2;
    let stroke = egui::Stroke::new(stroke_w, DRAG_LINE_COLOR);
    painter.line_segment(
        [to_screen(sx, sy), to_screen(pointer_mm.x, pointer_mm.y)],
        stroke,
    );
}

/// Result of pointer input handling: what's under the cursor and what was clicked.
#[derive(Clone, Debug, Default)]
pub struct ProtomatrixPointerState {
    /// Current element under the cursor (if any).
    pub hovered: Option<ProtomatrixTarget>,
    /// Element that was clicked this frame (set when primary released over a target).
    pub clicked: Option<ProtomatrixTarget>,
}

/// Handle pointer input: run hit-test, update hover and click state.
/// Call this each frame with pointer position in board mm, and whether primary button was clicked
/// (e.g. `response.clicked()`). Writes to `state`; caller should clear `state.clicked` after handling.
pub fn handle_pointer_input(
    config: &ProtomatrixConfig,
    pointer_mm: Option<egui::Vec2>,
    primary_clicked: bool,
    state: &mut ProtomatrixPointerState,
) {
    let current = pointer_mm.and_then(|p| hit_test(config, p.x, p.y));
    state.hovered = current.clone();
    if primary_clicked {
        state.clicked = current;
    } else {
        // Clear click so we don't reprocess it on subsequent frames
        state.clicked = None;
    }
}
