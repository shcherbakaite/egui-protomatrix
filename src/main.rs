use std::collections::HashMap;
use std::path::Path;

use eframe::egui;
use walkdir::WalkDir;
use crate::kicad9::Footprint;

mod footprint;
mod kicad9;

// --- PCB constants (mm), matching protomatrix.py ---
const PROTO_AREA: (i32, i32) = (60, 6);
const MATRIX_SIZE: i32 = 15;
const MATRIX_BREAK_EVERY: i32 = 10;
const MATRIX_BREAK_SIZE_MM: f32 = 0.508;   // 20 mil
const PROTO_PITCH_MM: f32 = 2.54;           // 100 mil
const MATRIX_V_PITCH_MM: f32 = 1.016;       // 40 mil
const PROTO_GAP_MM: f32 = 7.62;             // 300 mil
const TRACK_WIDTH_MM: f32 = 0.5;
const JUMPER_TRACK_OFFSET_X_MM: f32 = 0.381; // 15 mil
const OFFSET_100MIL_MM: f32 = 2.54;
const OFFSET_50MIL_MM: f32 = 1.27;
const OFFSET_30MIL_MM: f32 = 0.762;

const PROTO_PAD_DIA_MM: f32 = 1.9;
const PROTO_PAD_HOLE_MM: f32 = 1.2;
const VIA_DRILL_MM: f32 = 0.3;
const VIA_DIA_MM: f32 = 0.5;
const JUMPER_PAD_MM: f32 = 0.635;  // SolderJumper_Open_RoundedPad_0.635
const MOUNTING_HOLE_MM: f32 = 3.2;

/// Tracks drawn in background (semi-transparent grey).
fn track_color() -> egui::Color32 {
    egui::Color32::from_rgba_unmultiplied(128, 128, 128, 140)
}
const COPPER_COLOR: egui::Color32 = egui::Color32::from_rgb(0xb5, 0x73, 0x3c);
const MASK_COLOR: egui::Color32 = egui::Color32::from_rgb(0x18, 0x18, 0x18);
const OUTLINE_COLOR: egui::Color32 = egui::Color32::from_rgb(0x60, 0x60, 0x60);
/// MountingHole_3.2mm_M3: drill/substrate visible (np_thru_hole, no annular)
const HOLE_COLOR: egui::Color32 = egui::Color32::from_rgb(0x28, 0x20, 0x1c);

/// Visible rectangle in board mm (for culling). Adds margin so elements at edges aren't clipped.
fn visible_rect_mm(
    rect: &egui::Rect,
    origin: egui::Pos2,
    pan: egui::Vec2,
    scale: f32,
    margin_mm: f32,
) -> (f32, f32, f32, f32) {
    let inv = 1.0 / scale;
    let vx_min = (rect.left() - origin.x - pan.x) * inv - margin_mm;
    let vx_max = (rect.right() - origin.x - pan.x) * inv + margin_mm;
    let vy_min = (rect.top() - origin.y - pan.y) * inv - margin_mm;
    let vy_max = (rect.bottom() - origin.y - pan.y) * inv + margin_mm;
    (vx_min, vx_max, vy_min, vy_max)
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

// SolderJumper_Open_RoundedPad_0.635_C.kicad_mod — F.Cu polygons (mm, footprint origin at center; KiCad y up)
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

fn main() -> eframe::Result<()> {
    if std::env::args().any(|a| a == "--check-footprints") {
        check_footprints();
        return Ok(());
    }

    let path = Path::new("/usr/share/kicad/footprints/Package_DIP.pretty/DIP-10_W10.16mm.kicad_mod");
    let _f1 = footprint::load_footprint(path);

    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1200.0, 800.0]),
        ..Default::default()
    };

    eframe::run_native(
        "Proto Matrix PCB",
        native_options,
        Box::new(|cc| Ok(Box::new(CanvasApp::new(cc)))),
    )
}

const ZOOM_MIN: f32 = 0.15;
const ZOOM_MAX: f32 = 25.0;
/// Scroll-to-zoom: scale factor per point of smooth scroll (reduces stutter vs raw_scroll_delta).
const ZOOM_SENSITIVITY: f32 = 0.0008;

/// Loaded Package_DIP library; one footprint is drawn for demo.
const PACKAGE_DIP_PATH: &str = "/usr/share/kicad/footprints/Package_DIP.pretty/";
//const PACKAGE_DIP_PATH: &str = "/usr/share/kicad/footprints/Resistor_THT.pretty";


/// Single footprint to load when directory load fails (any .kicad_mod name).
const FALLBACK_FOOTPRINT: &str = "DIP-4_W7.62mm";

/// Half-size of the grab rect around the DIP package (mm), for hit-test when dragging.
/// Use a square large enough to contain the footprint at any rotation (0°: ~8×15, so max 15).
const DIP_GRAB_RECT_HALF_MM: f32 = 16.0;

struct CanvasApp {
    pan: egui::Vec2,
    zoom: f32,
    /// Loaded footprints from Package_DIP (name -> Footprint).
    dip_lib: Option<HashMap<String, Footprint>>,
    /// Package position in mm. None = use default (to the right of board).
    dip_pos_mm: Option<egui::Vec2>,
    /// Package rotation in degrees (counterclockwise). 0 = default orientation.
    dip_rotation_deg: f32,
    /// True while left-dragging the package (so we keep moving it until release).
    dragging_package: bool,
}

impl CanvasApp {
    fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        let dip_lib = load_package_dip();
        if let Some(ref lib) = dip_lib {
            print_footprint_info(lib);
        } else {
            println!("[footprint] no library loaded from {}", PACKAGE_DIP_PATH);
        }
        Self {
            pan: egui::Vec2::ZERO,
            zoom: 1.0,
            dip_lib,
            dip_pos_mm: None,
            dip_rotation_deg: 0.0,
            dragging_package: false,
        }
    }
}

/// Print summary of loaded footprint library and the first footprint (the one we draw).
fn print_footprint_info(lib: &HashMap<String, Footprint>) {
    println!("[footprint] loaded {} footprint(s) from {}", lib.len(), PACKAGE_DIP_PATH);
    if let Some((name, fp)) = lib.iter().next() {
        let pad_count = fp.pads().count();
        let graphic_count = fp.graphics().count();
        println!(
            "[footprint] drawing '{}': elements={} pads={} graphics={}",
            name,
            fp.elements.len(),
            pad_count,
            graphic_count
        );
    }
}

/// Load Package_DIP footprint library from system path.
fn load_package_dip() -> Option<HashMap<String, Footprint>> {
    let mut candidates: Vec<std::path::PathBuf> = vec![
        PACKAGE_DIP_PATH.into(),
        "/usr/share/kicad/footprints".into(),
    ];
    if let Ok(env_path) = std::env::var("KICAD_FOOTPRINTS") {
        candidates.insert(0, Path::new(&env_path).join("Package_DIP.pretty"));
    }
    for path in candidates {
        match footprint::load_footprint_dir(&path) {
            Ok(lib) if !lib.is_empty() => {
                println!("[footprint] loaded from {}", path.display());
                return Some(lib);
            }
            Ok(_) => {
                println!("[footprint] dir empty: {}", path.display());
            }
            Err(e) => {
                println!("[footprint] dir failed {}: {}", path.display(), e);
            }
        }
        // Fallback: load one footprint from this candidate.
        let single = path.join(format!("{}.kicad_mod", FALLBACK_FOOTPRINT));
        if let Ok(module) = footprint::load_footprint(&single) {
            println!("[footprint] loaded single from {}", single.display());
            let mut map = HashMap::new();
            map.insert(module.name.clone(), module);
            return Some(map);
        }
    }
    None
}

/// Run footprint parse check: walk /usr/share/kicad/footprints, try to parse each .kicad_mod,
/// report errors grouped by unknown element type. Run with: cargo run -- --check-footprints
fn check_footprints() {
    let root = Path::new("/usr/share/kicad/footprints");
    if !root.exists() {
        eprintln!("footprints dir not found: {}", root.display());
        return;
    }

    let mut ok = 0u64;
    let mut fail = 0u64;
    let mut errors: HashMap<String, Vec<(String, String)>> = HashMap::new();

    for entry in WalkDir::new(root).follow_links(true).max_depth(2) {
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                eprintln!("walk error: {}", e);
                continue;
            }
        };
        let path = entry.path();
        if path.is_file()
            && path
                .extension()
                .and_then(std::ffi::OsStr::to_str)
                .map_or(false, |e| e == "kicad_mod")
        {
            match footprint::load_footprint(path) {
                Ok(_fp) => ok += 1,
                Err(e) => {
                    fail += 1;
                    let err_str = format!("{:?}", e);
                    let key = extract_unknown_element(&err_str)
                        .map(String::from)
                        .unwrap_or_else(|| "other".to_string());
                    errors.entry(key).or_default().push((
                        path.display().to_string(),
                        err_str,
                    ));
                }
            }
        }
    }

    println!("Parse results:");
    println!("  OK: {}", ok);
    println!("  Fail: {}", fail);
    if !errors.is_empty() {
        println!("\nErrors by type:");
        let mut keys: Vec<_> = errors.keys().collect();
        keys.sort();
        for key in keys {
            let v = &errors[key];
            println!("  {} ({} files):", key, v.len());
            for (path, err) in v.iter().take(3) {
                println!("    {} -> {}", path, err);
            }
            if v.len() > 3 {
                println!("    ... and {} more", v.len() - 3);
            }
        }
    }
}

fn extract_unknown_element(err: &str) -> Option<&str> {
    let prefix = "unknown element in module: ";
    if let Some(start) = err.find(prefix) {
        let rest = &err[start + prefix.len()..];
        if let Some(end) = rest.find('"') {
            return Some(rest[..end].trim());
        }
    }
    None
}

impl eframe::App for CanvasApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.set_min_size(ui.available_size());
            let (x_min, x_max, y_min, y_max) = board_bounds_mm();
            let board_width_mm = x_max - x_min + 4.0;
            let board_height_mm = y_max - y_min + 4.0;
            let board_center_mm = egui::vec2(
                (x_min + x_max) / 2.0,
                (y_min + y_max) / 2.0,
            );

            let rect = ui.available_rect_before_wrap();
            let base_scale = (rect.width() / board_width_mm)
                .min(rect.height() / board_height_mm)
                .max(0.1);

            let sense = egui::Sense::drag().union(egui::Sense::hover());
            let response = ui.allocate_rect(rect, sense);

            if ui.input(|i| i.pointer.primary_released()) {
                self.dragging_package = false;
            }

            if self.dip_lib.is_some() && ui.input(|i| i.key_pressed(egui::Key::R)) {
                self.dip_rotation_deg = (self.dip_rotation_deg + 90.0).rem_euclid(360.0);
            }

            // Pan: middle or right mouse drag (round to pixel to avoid sub-pixel jitter)
            if response.dragged_by(egui::PointerButton::Middle)
                || response.dragged_by(egui::PointerButton::Secondary)
            {
                let delta = response.drag_delta();
                self.pan += egui::vec2(delta.x.round(), delta.y.round());
            }

            // Zoom: scroll wheel, zoom toward cursor
            if response.hovered() {
                let scroll = ui.input(|i| i.smooth_scroll_delta.y);
                if scroll != 0.0 {
                    let zoom_delta = (1.0 - scroll * ZOOM_SENSITIVITY).clamp(0.9, 1.1);
                    let new_zoom = (self.zoom * zoom_delta).clamp(ZOOM_MIN, ZOOM_MAX);
                    if let Some(pos) = response.hover_pos() {
                        let scale_before = base_scale * self.zoom;
                        let origin_before = rect.center() - board_center_mm * scale_before;
                        let world_x = (pos.x - origin_before.x - self.pan.x) / scale_before;
                        let world_y = (pos.y - origin_before.y - self.pan.y) / scale_before;
                        self.zoom = new_zoom;
                        let scale_after = base_scale * self.zoom;
                        let origin_after = rect.center() - board_center_mm * scale_after;
                        self.pan.x = pos.x - origin_after.x - world_x * scale_after;
                        self.pan.y = pos.y - origin_after.y - world_y * scale_after;
                    } else {
                        self.zoom = new_zoom;
                    }
                }
            }

            // Keep pan pixel-aligned to reduce stutter
            self.pan = egui::vec2(self.pan.x.round(), self.pan.y.round());

            // Recompute scale and origin from current zoom for this frame's drawing
            let scale = base_scale * self.zoom;
            let origin = rect.center() - board_center_mm * scale;

            let default_dip_mm = egui::vec2(x_max + 18.0, 0.0);
            let dip_pos_mm = self.dip_pos_mm.unwrap_or(default_dip_mm);

            // Pointer position in world mm (for package grab hit-test)
            let pointer_mm = response.hover_pos().map(|pos| {
                egui::vec2(
                    (pos.x - origin.x - self.pan.x) / scale,
                    (pos.y - origin.y - self.pan.y) / scale,
                )
            });
            let package_rect_min = egui::vec2(
                dip_pos_mm.x - DIP_GRAB_RECT_HALF_MM,
                dip_pos_mm.y - DIP_GRAB_RECT_HALF_MM,
            );
            let package_rect_max = egui::vec2(
                dip_pos_mm.x + DIP_GRAB_RECT_HALF_MM,
                dip_pos_mm.y + DIP_GRAB_RECT_HALF_MM,
            );
            let package_visible = self.dip_lib.is_some();
            let pointer_over_package = package_visible
                && pointer_mm.map_or(false, |p| {
                    p.x >= package_rect_min.x && p.x <= package_rect_max.x
                        && p.y >= package_rect_min.y && p.y <= package_rect_max.y
                });

            if response.dragged_by(egui::PointerButton::Primary) {
                let delta = response.drag_delta();
                let delta_mm = egui::vec2(delta.x / scale, delta.y / scale);
                if (self.dragging_package || pointer_over_package) && package_visible {
                    if !self.dragging_package {
                        self.dip_pos_mm = Some(dip_pos_mm);
                        self.dragging_package = true;
                    }
                    if let Some(ref mut pos) = self.dip_pos_mm {
                        *pos += delta_mm;
                    }
                }
            }

            let to_screen = |x_mm: f32, y_mm: f32| -> egui::Pos2 {
                let v = origin + self.pan + egui::vec2(x_mm * scale, y_mm * scale);
                egui::pos2(v.x, v.y)
            };

            let view = visible_rect_mm(&rect, origin, self.pan, scale, 5.0);

            ui.set_clip_rect(rect);
            let painter = ui.painter_at(rect);

            let tl = to_screen(x_min - 2.0, y_min - 2.0);
            let br = to_screen(x_max + 2.0, y_max + 2.0);
            let board_rect = egui::Rect::from_min_max(tl, br);

            painter.rect_filled(board_rect, egui::CornerRadius::ZERO, MASK_COLOR);
            painter.rect_stroke(
                board_rect,
                egui::CornerRadius::ZERO,
                egui::Stroke::new(1.5, OUTLINE_COLOR),
                egui::StrokeKind::Outside,
            );
            draw_tracks(&painter, &to_screen, scale, view);
            draw_proto_pads(&painter, &to_screen, scale, view);
            draw_matrix_areas(&painter, &to_screen, scale, view);
            draw_y_link_vias(&painter, &to_screen, scale, view);
            draw_mounting_holes(&painter, &to_screen, scale, view);

            // Draw one footprint (any from the library); position is draggable.
            if let Some(ref lib) = self.dip_lib {
                if let Some((_name, fp)) = lib.iter().next() {
                    let view_dip = (
                        view.0,
                        view.1.max(dip_pos_mm.x + 25.0),
                        view.2.min(dip_pos_mm.y - 12.0),
                        view.3.max(dip_pos_mm.y + 12.0),
                    );
                    footprint::draw_footprint(
                        &painter,
                        &to_screen,
                        scale,
                        view_dip,
                        fp,
                        dip_pos_mm.x,
                        dip_pos_mm.y,
                        self.dip_rotation_deg as f64,
                    );
                }
            }

            if self.dragging_package {
                ctx.set_cursor_icon(egui::CursorIcon::Grabbing);
            } else if pointer_over_package {
                ctx.set_cursor_icon(egui::CursorIcon::Grab);
            } else if response.dragged() {
                ctx.set_cursor_icon(egui::CursorIcon::Grabbing);
            } else if response.hovered() {
                ctx.set_cursor_icon(egui::CursorIcon::Grab);
            }
        });
    }
}

fn board_bounds_mm() -> (f32, f32, f32, f32) {
    let x_max = (PROTO_AREA.0 as f32 - 1.0) * PROTO_PITCH_MM + 5.0;
    let x_min = -OFFSET_100MIL_MM - 15.0 * OFFSET_30MIL_MM - 2.0;
    let y_lower_matrix_end =
        PROTO_AREA.1 as f32 * PROTO_PITCH_MM
            + (MATRIX_SIZE as f32 * MATRIX_V_PITCH_MM)
            + (MATRIX_SIZE / MATRIX_BREAK_EVERY) as f32 * MATRIX_BREAK_SIZE_MM;
    let y_upper_matrix_top = -PROTO_GAP_MM
        - PROTO_AREA.1 as f32 * PROTO_PITCH_MM
        - (MATRIX_SIZE as f32 * MATRIX_V_PITCH_MM)
        - (MATRIX_SIZE / MATRIX_BREAK_EVERY) as f32 * MATRIX_BREAK_SIZE_MM;
    (x_min, x_max, y_upper_matrix_top - 2.0, y_lower_matrix_end + 2.0)
}

fn matrix_gradient(_i: i32, j: i32) -> (f32, f32) {
    let mut dy = MATRIX_V_PITCH_MM;
    if (j % MATRIX_BREAK_EVERY) == 0 && j != 0 {
        dy += MATRIX_BREAK_SIZE_MM;
    }
    (PROTO_PITCH_MM, dy)
}

/// Y position of row j in lower matrix (origin at first row).
fn lower_matrix_row_y(j: i32) -> f32 {
    let (_ox, oy) = lower_matrix_offset();
    let mut y = oy;
    for k in 0..j {
        let (_, dy) = matrix_gradient(0, k);
        y += dy;
    }
    y
}

/// Y position of row j in upper matrix (neg_dy gradient, origin at first row).
fn upper_matrix_row_y(j: i32) -> f32 {
    let (_, oy) = upper_matrix_offset();
    let mut y = oy;
    for k in 0..j {
        let (_, dy) = matrix_gradient(0, k);
        y -= dy;
    }
    y
}

fn lower_matrix_offset() -> (f32, f32) {
    (0.0, PROTO_AREA.1 as f32 * PROTO_PITCH_MM)
}

fn upper_matrix_offset() -> (f32, f32) {
    (0.0, -PROTO_AREA.1 as f32 * PROTO_PITCH_MM - PROTO_GAP_MM)
}

fn draw_proto_pads(
    painter: &egui::Painter,
    to_screen: &impl Fn(f32, f32) -> egui::Pos2,
    scale: f32,
    view: (f32, f32, f32, f32),
) {
    let r = PROTO_PAD_DIA_MM / 2.0;
    for i in 0..PROTO_AREA.0 {
        for j in 0..PROTO_AREA.1 {
            let x = i as f32 * PROTO_PITCH_MM;
            let y = j as f32 * PROTO_PITCH_MM;
            if !in_view(x, y, view) {
                continue;
            }
            let c = to_screen(x, y);
            painter.circle_filled(c, r * scale, COPPER_COLOR);
            painter.circle_filled(c, (PROTO_PAD_HOLE_MM / 2.0) * scale, MASK_COLOR);
        }
    }
    for i in 0..PROTO_AREA.0 {
        for j in 0..PROTO_AREA.1 {
            let x = i as f32 * PROTO_PITCH_MM;
            let y = -PROTO_GAP_MM - j as f32 * PROTO_PITCH_MM;
            if !in_view(x, y, view) {
                continue;
            }
            let c = to_screen(x, y);
            painter.circle_filled(c, r * scale, COPPER_COLOR);
            painter.circle_filled(c, (PROTO_PAD_HOLE_MM / 2.0) * scale, MASK_COLOR);
        }
    }
}

/// Draw solder jumper pad at board position (cx, cy) mm. Footprint origin center; KiCad y-up -> board y-down.
fn draw_solder_jumper_pad(
    painter: &egui::Painter,
    to_screen: &impl Fn(f32, f32) -> egui::Pos2,
    cx: f32,
    cy: f32,
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
        COPPER_COLOR,
        egui::Stroke::NONE,
    ));
    painter.add(egui::Shape::convex_polygon(
        left_pts,
        COPPER_COLOR,
        egui::Stroke::NONE,
    ));
}

fn draw_matrix_areas(
    painter: &egui::Painter,
    to_screen: &impl Fn(f32, f32) -> egui::Pos2,
    scale: f32,
    view: (f32, f32, f32, f32),
) {
    let (ox, _) = lower_matrix_offset();
    for j in 0..MATRIX_SIZE {
        let y = lower_matrix_row_y(j);
        for i in 0..PROTO_AREA.0 {
            let x = ox + i as f32 * PROTO_PITCH_MM;
            if !in_view(x, y, view) {
                continue;
            }
            draw_solder_jumper_pad(painter, to_screen, x, y);
            let via_x = x - PROTO_PITCH_MM / 2.0;
            let vp = to_screen(via_x, y);
            painter.circle_filled(vp, (VIA_DIA_MM / 2.0) * scale, COPPER_COLOR);
            painter.circle_filled(vp, (VIA_DRILL_MM / 2.0) * scale, MASK_COLOR);
        }
    }
    let (ox, _) = upper_matrix_offset();
    for j in 0..MATRIX_SIZE {
        let y = upper_matrix_row_y(j);
        for i in 0..PROTO_AREA.0 {
            let x = ox + i as f32 * PROTO_PITCH_MM;
            if !in_view(x, y, view) {
                continue;
            }
            draw_solder_jumper_pad(painter, to_screen, x, y);
            let via_x = x - PROTO_PITCH_MM / 2.0;
            let vp = to_screen(via_x, y);
            painter.circle_filled(vp, (VIA_DIA_MM / 2.0) * scale, COPPER_COLOR);
            painter.circle_filled(vp, (VIA_DRILL_MM / 2.0) * scale, MASK_COLOR);
        }
    }
}

fn draw_tracks(
    painter: &egui::Painter,
    to_screen: &impl Fn(f32, f32) -> egui::Pos2,
    scale: f32,
    view: (f32, f32, f32, f32),
) {
    let stroke_w = TRACK_WIDTH_MM * scale;
    let stroke = egui::Stroke::new(stroke_w, track_color());
    let half = MATRIX_SIZE / 2;

    for i in 0..PROTO_AREA.0 {
        for j in 0..PROTO_AREA.1 - 1 {
            let x = i as f32 * PROTO_PITCH_MM;
            let y0 = j as f32 * PROTO_PITCH_MM;
            let y1 = (j + 1) as f32 * PROTO_PITCH_MM;
            if !segment_in_view(x, y0, x, y1, view) {
                continue;
            }
            painter.line_segment([to_screen(x, y0), to_screen(x, y1)], stroke);
        }
        for j in 0..PROTO_AREA.1 - 1 {
            let x = i as f32 * PROTO_PITCH_MM;
            let y0 = -PROTO_GAP_MM - j as f32 * PROTO_PITCH_MM;
            let y1 = -PROTO_GAP_MM - (j + 1) as f32 * PROTO_PITCH_MM;
            if !segment_in_view(x, y0, x, y1, view) {
                continue;
            }
            painter.line_segment([to_screen(x, y0), to_screen(x, y1)], stroke);
        }
    }

    for j in 0..MATRIX_SIZE {
        let row_y = lower_matrix_row_y(j);
        let link_x = -OFFSET_100MIL_MM - (j % half) as f32 * OFFSET_30MIL_MM;
        let end_x = (PROTO_AREA.0 as f32 - 1.0) * PROTO_PITCH_MM - OFFSET_50MIL_MM;
        if !segment_in_view(link_x, row_y, end_x, row_y, view) {
            continue;
        }
        painter.line_segment(
            [to_screen(link_x, row_y), to_screen(end_x, row_y)],
            stroke,
        );
    }
    for j in 0..MATRIX_SIZE {
        let row_y = upper_matrix_row_y(j);
        let link_x = -OFFSET_100MIL_MM - (j % half) as f32 * OFFSET_30MIL_MM;
        let end_x = (PROTO_AREA.0 as f32 - 1.0) * PROTO_PITCH_MM - OFFSET_50MIL_MM;
        if !segment_in_view(link_x, row_y, end_x, row_y, view) {
            continue;
        }
        painter.line_segment(
            [to_screen(link_x, row_y), to_screen(end_x, row_y)],
            stroke,
        );
    }

    let center_y = -PROTO_GAP_MM / 2.0;
    for j in 0..MATRIX_SIZE {
        let row_y = lower_matrix_row_y(j);
        let link_x = -OFFSET_100MIL_MM - (j % half) as f32 * OFFSET_30MIL_MM;
        if !segment_in_view(link_x, row_y, link_x, center_y, view) {
            continue;
        }
        painter.line_segment(
            [to_screen(link_x, row_y), to_screen(link_x, center_y)],
            stroke,
        );
    }
    for j in 0..MATRIX_SIZE {
        let row_y = upper_matrix_row_y(j);
        let link_x = -OFFSET_100MIL_MM - (j % half) as f32 * OFFSET_30MIL_MM;
        if !segment_in_view(link_x, row_y, link_x, center_y, view) {
            continue;
        }
        painter.line_segment(
            [to_screen(link_x, row_y), to_screen(link_x, center_y)],
            stroke,
        );
    }

    for j in 0..MATRIX_SIZE - 1 {
        let y0 = lower_matrix_row_y(j);
        let y1 = lower_matrix_row_y(j + 1);
        for i in 0..PROTO_AREA.0 {
            let px = i as f32 * PROTO_PITCH_MM + JUMPER_TRACK_OFFSET_X_MM;
            if !segment_in_view(px, y0, px, y1, view) {
                continue;
            }
            painter.line_segment([to_screen(px, y0), to_screen(px, y1)], stroke);
        }
    }
    for j in 0..MATRIX_SIZE - 1 {
        let y0 = upper_matrix_row_y(j);
        let y1 = upper_matrix_row_y(j + 1);
        for i in 0..PROTO_AREA.0 {
            let px = i as f32 * PROTO_PITCH_MM + JUMPER_TRACK_OFFSET_X_MM;
            if !segment_in_view(px, y0, px, y1, view) {
                continue;
            }
            painter.line_segment([to_screen(px, y0), to_screen(px, y1)], stroke);
        }
    }

    for i in 0..PROTO_AREA.0 {
        let x = i as f32 * PROTO_PITCH_MM;
        let y_bottom = (PROTO_AREA.1 - 1) as f32 * PROTO_PITCH_MM;
        let y_half = y_bottom + PROTO_PITCH_MM / 2.0;
        let y_knob = y_half + 0.381;
        let y_end = PROTO_AREA.1 as f32 * PROTO_PITCH_MM;
        let cx = x + JUMPER_TRACK_OFFSET_X_MM;
        if segment_in_view(x, y_bottom, x, y_half, view) {
            painter.line_segment([to_screen(x, y_bottom), to_screen(x, y_half)], stroke);
        }
        if segment_in_view(x, y_half, cx, y_knob, view) {
            painter.line_segment([to_screen(x, y_half), to_screen(cx, y_knob)], stroke);
        }
        if segment_in_view(cx, y_knob, cx, y_end, view) {
            painter.line_segment([to_screen(cx, y_knob), to_screen(cx, y_end)], stroke);
        }
    }
    for i in 0..PROTO_AREA.0 {
        let x = i as f32 * PROTO_PITCH_MM;
        let y_top_pad = -PROTO_GAP_MM;
        let y_half = y_top_pad - PROTO_PITCH_MM / 2.0;
        let y_knob = y_half - 0.381;
        let y_end = -PROTO_GAP_MM - PROTO_AREA.1 as f32 * PROTO_PITCH_MM;
        let cx = x + JUMPER_TRACK_OFFSET_X_MM;
        if segment_in_view(x, y_top_pad, x, y_half, view) {
            painter.line_segment([to_screen(x, y_top_pad), to_screen(x, y_half)], stroke);
        }
        if segment_in_view(x, y_half, cx, y_knob, view) {
            painter.line_segment([to_screen(x, y_half), to_screen(cx, y_knob)], stroke);
        }
        if segment_in_view(cx, y_knob, cx, y_end, view) {
            painter.line_segment([to_screen(cx, y_knob), to_screen(cx, y_end)], stroke);
        }
    }
}

/// Vias at Y link positions (j >= half) — drawn on top of tracks.
fn draw_y_link_vias(
    painter: &egui::Painter,
    to_screen: &impl Fn(f32, f32) -> egui::Pos2,
    scale: f32,
    view: (f32, f32, f32, f32),
) {
    let half = MATRIX_SIZE / 2;
    for j in half..MATRIX_SIZE {
        let row_y = lower_matrix_row_y(j);
        let link_x = -OFFSET_100MIL_MM - (j % half) as f32 * OFFSET_30MIL_MM;
        if !in_view(link_x, row_y, view) {
            continue;
        }
        let vp = to_screen(link_x, row_y);
        painter.circle_filled(vp, (VIA_DIA_MM / 2.0) * scale, COPPER_COLOR);
        painter.circle_filled(vp, (VIA_DRILL_MM / 2.0) * scale, MASK_COLOR);
    }
    for j in half..MATRIX_SIZE {
        let row_y = upper_matrix_row_y(j);
        let link_x = -OFFSET_100MIL_MM - (j % half) as f32 * OFFSET_30MIL_MM;
        if !in_view(link_x, row_y, view) {
            continue;
        }
        let vp = to_screen(link_x, row_y);
        painter.circle_filled(vp, (VIA_DIA_MM / 2.0) * scale, COPPER_COLOR);
        painter.circle_filled(vp, (VIA_DRILL_MM / 2.0) * scale, MASK_COLOR);
    }
}

/// MountingHole_3.2mm_M3: three holes on center line (BuildMountingHoles from protomatrix.py).
fn draw_mounting_holes(
    painter: &egui::Painter,
    to_screen: &impl Fn(f32, f32) -> egui::Pos2,
    scale: f32,
    view: (f32, f32, f32, f32),
) {
    let center_y = -PROTO_GAP_MM / 2.0;
    let r = (MOUNTING_HOLE_MM / 2.0) * scale; // 3.2mm diameter from footprint
    let positions = [
        PROTO_PITCH_MM / 2.0,
        (PROTO_AREA.0 as f32 - 1.0) * PROTO_PITCH_MM / 2.0,
        (PROTO_AREA.0 as f32 - 1.0) * PROTO_PITCH_MM - PROTO_PITCH_MM / 2.0,
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
