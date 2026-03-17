use std::collections::HashMap;
use std::path::{Path, PathBuf};

use eframe::egui;
use serde::{Deserialize, Serialize};
use walkdir::WalkDir;

use crate::kicad9::Footprint;
use crate::protomatrix::{
    ProtoSide, ProtomatrixConfig, ProtomatrixPointerState, ProtomatrixTarget,
};
use crate::router::{AutorouteResult, ColumnKey, Connection, JumperState};

/// Ensure path has .json extension (for Linux/GTK which doesn't append automatically).
fn ensure_json_extension(path: PathBuf) -> PathBuf {
    if path.extension().map_or(true, |e| e != "json") {
        path.with_extension("json")
    } else {
        path
    }
}

/// Canonical key string for net color lookup (e.g. "L3" for Lower col 3, "U5" for Upper col 5).
pub fn column_key_to_string(col: &ColumnKey) -> String {
    let side = match col.side {
        ProtoSide::Lower => 'L',
        ProtoSide::Upper => 'U',
    };
    format!("{}{}", side, col.col)
}

/// Board file format for save/load.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BoardFile {
    pub config: ProtomatrixConfig,
    pub connections: Vec<Connection>,
    #[serde(default)]
    pub net_names: HashMap<usize, String>,
    /// Net color overrides keyed by canonical column key (e.g. "L3", "U5").
    #[serde(default)]
    pub net_colors: HashMap<String, [u8; 4]>,
}

mod footprint;
mod kicad9;
mod protomatrix;
mod router;

const MASK_COLOR: egui::Color32 = egui::Color32::from_rgb(0x18, 0x18, 0x18);
const OUTLINE_COLOR: egui::Color32 = egui::Color32::from_rgb(0x60, 0x60, 0x60);

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
    /// Protomatrix layout configuration.
    protomatrix_config: ProtomatrixConfig,
    /// Hover and click state for protomatrix elements.
    protomatrix_state: ProtomatrixPointerState,
    /// Last clicked target (for status display; cleared when new click).
    last_clicked: Option<ProtomatrixTarget>,
    /// Pad-to-pad connections (user drags from pad to pad to connect).
    connections: Vec<Connection>,
    /// Pad we started dragging from (while creating a connection); line drawn to pointer until release.
    connection_drag_source: Option<ProtomatrixTarget>,
    /// Autorouter result: which jumpers to close.
    jumper_state: Option<JumperState>,
    /// Autorouter error message (e.g. too many nets).
    autoroute_error: Option<String>,
    /// User-assigned names for nets (net index -> name).
    net_names: HashMap<usize, String>,
    /// Net color overrides keyed by canonical column key (e.g. "L3", "U5").
    net_colors: HashMap<String, [u8; 4]>,
    /// Net index of the selected element (pad or jumper with a net).
    selected_net: Option<usize>,
    /// When Some(net_index), show rename dialog for that net.
    show_rename_net_dialog: Option<usize>,
    /// Buffer for the rename net dialog.
    rename_net_name: String,
    /// Column for context menu (captured when menu opens, so Disconnect works when pointer is over menu).
    context_menu_column: Option<ColumnKey>,
    /// Current file path when board was opened/saved.
    file_path: Option<std::path::PathBuf>,
    /// Show Board -> Set Size dialog.
    show_set_size_dialog: bool,
    /// Edited values in Set Size dialog (initialized when dialog opens).
    set_size_proto_cols: i32,
    set_size_proto_rows: i32,
    set_size_matrix_size: i32,
    set_size_break_every: i32,
    /// Apply was clicked in Set Size dialog (process after window runs).
    set_size_apply_clicked: bool,
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
            protomatrix_config: ProtomatrixConfig::default(),
            protomatrix_state: ProtomatrixPointerState::default(),
            last_clicked: None,
            connections: Vec::new(),
            connection_drag_source: None,
            jumper_state: None,
            autoroute_error: None,
            net_names: HashMap::new(),
            net_colors: HashMap::new(),
            selected_net: None,
            show_rename_net_dialog: None,
            rename_net_name: String::new(),
            context_menu_column: None,
            file_path: None,
            show_set_size_dialog: false,
            set_size_proto_cols: 63,
            set_size_proto_rows: 5,
            set_size_matrix_size: 15,
            set_size_break_every: 10,
            set_size_apply_clicked: false,
        }
    }

    fn load_board(&mut self, path: &std::path::Path) -> Result<(), String> {
        let s = std::fs::read_to_string(path)
            .map_err(|e| format!("Read failed: {}", e))?;
        let board: BoardFile = serde_json::from_str(&s)
            .map_err(|e| format!("Parse failed: {}", e))?;
        self.protomatrix_config = board.config;
        self.connections = board.connections;
        self.net_names = board.net_names;
        self.net_colors = board.net_colors;
        self.connection_drag_source = None;
        self.selected_net = None;
        self.autoroute_error = None;
        match router::autoroute(&self.protomatrix_config, &self.connections) {
            AutorouteResult::Ok(js) => self.jumper_state = Some(js),
            AutorouteResult::Err(e) => {
                self.autoroute_error = Some(e);
                self.jumper_state = None;
            }
        }
        self.file_path = Some(path.to_path_buf());
        Ok(())
    }

    fn save_board(&self, path: &std::path::Path) -> Result<(), String> {
        let board = BoardFile {
            config: self.protomatrix_config.clone(),
            connections: self.connections.clone(),
            net_names: self.net_names.clone(),
            net_colors: self.net_colors.clone(),
        };
        let s = serde_json::to_string_pretty(&board)
            .map_err(|e| format!("Serialize failed: {}", e))?;
        std::fs::write(path, s).map_err(|e| format!("Write failed: {}", e))?;
        Ok(())
    }

    fn close_board(&mut self) {
        self.protomatrix_config = ProtomatrixConfig::default();
        self.connections.clear();
        self.net_names.clear();
        self.net_colors.clear();
        self.context_menu_column = None;
        self.connection_drag_source = None;
        self.jumper_state = None;
        self.autoroute_error = None;
        self.file_path = None;
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

/// Column (side, col) from a target; None for MatrixRow.
fn column_from_target(target: &ProtomatrixTarget) -> Option<ColumnKey> {
    match target {
        ProtomatrixTarget::Pad { side, col, .. } => Some(ColumnKey {
            side: *side,
            col: *col,
        }),
        ProtomatrixTarget::SolderJumper { side, col, .. } => Some(ColumnKey {
            side: *side,
            col: *col,
        }),
        ProtomatrixTarget::MatrixRow { .. } => None,
    }
}

/// Disconnect the given column from its net. Removes connections involving that column,
/// and adds new connections between the remaining columns so the rest of the net stays connected.
fn disconnect_column(connections: &mut Vec<Connection>, col: ColumnKey) {
    // Collect pads from the "other" columns (neighbors) in connections involving col
    let mut neighbor_pads: HashMap<ColumnKey, ProtomatrixTarget> = HashMap::new();
    for c in connections.iter() {
        if ColumnKey::from_pad(&c.a) == Some(col) {
            if let Some(k) = ColumnKey::from_pad(&c.b) {
                neighbor_pads.entry(k).or_insert_with(|| c.b.clone());
            }
        } else if ColumnKey::from_pad(&c.b) == Some(col) {
            if let Some(k) = ColumnKey::from_pad(&c.a) {
                neighbor_pads.entry(k).or_insert_with(|| c.a.clone());
            }
        }
    }
    // Remove connections involving the column
    connections.retain(|c| {
        ColumnKey::from_pad(&c.a) != Some(col) && ColumnKey::from_pad(&c.b) != Some(col)
    });
    // Add connections to keep the remaining columns in one net (spanning tree: first to all others)
    let pads: Vec<ProtomatrixTarget> = neighbor_pads.into_values().collect();
    if pads.len() >= 2 {
        let first = pads[0].clone();
        for other in pads.iter().skip(1) {
            connections.push(Connection::new(first.clone(), other.clone()));
        }
    }
}

/// Get net index for a target (pad or solder jumper) if it belongs to a net.
fn net_index_for_target(
    target: &ProtomatrixTarget,
    jumper_state: Option<&JumperState>,
    matrix_size: i32,
) -> Option<usize> {
    let js = jumper_state?;
    match target {
        ProtomatrixTarget::Pad { side, col, .. } => {
            js.net_for_column(*side, *col, matrix_size)
        }
        ProtomatrixTarget::SolderJumper { side, col, row } => js.net_index(*side, *col, *row),
        ProtomatrixTarget::MatrixRow { .. } => None,
    }
}

fn protomatrix_target_label(t: &ProtomatrixTarget) -> String {
    let side = |s: ProtoSide| match s {
        ProtoSide::Upper => "upper",
        ProtoSide::Lower => "lower",
    };
    match t {
        ProtomatrixTarget::Pad { side: s, col, row } => {
            format!("pad {} ({},{})", side(*s), col, row)
        }
        ProtomatrixTarget::MatrixRow { side: s, row } => {
            format!("matrix row {} [{}]", row, side(*s))
        }
        ProtomatrixTarget::SolderJumper { side: s, col, row } => {
            format!("jumper ({},{}) [{}]", col, row, side(*s))
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
        // Menu bar
        egui::TopBottomPanel::top("menu_bar").show(ctx, |ui| {
            egui::MenuBar::new().ui(ui, |ui| {
                ui.menu_button("File", |ui| {
                    if ui.button("Open…").clicked() {
                        ui.close();
                        if let Some(path) = rfd::FileDialog::new()
                            .add_filter("Board files", &["json"])
                            .add_filter("All files", &["*"])
                            .pick_file()
                        {
                            if let Err(e) = self.load_board(&path) {
                                eprintln!("Load error: {}", e);
                            }
                        }
                    }
                    if ui.button("Save").clicked() {
                        ui.close();
                        let path = self.file_path.clone().or_else(|| {
                            rfd::FileDialog::new()
                                .add_filter("Board files", &["json"])
                                .set_file_name("board.json")
                                .save_file()
                        });
                        if let Some(path) = path {
                            let path = ensure_json_extension(path);
                            if let Err(e) = self.save_board(&path) {
                                eprintln!("Save error: {}", e);
                            } else {
                                self.file_path = Some(path);
                            }
                        }
                    }
                    if ui.button("Save As…").clicked() {
                        ui.close();
                        if let Some(path) = rfd::FileDialog::new()
                            .add_filter("Board files", &["json"])
                            .set_file_name("board.json")
                            .save_file()
                        {
                            let path = ensure_json_extension(path);
                            if let Err(e) = self.save_board(&path) {
                                eprintln!("Save error: {}", e);
                            } else {
                                self.file_path = Some(path);
                            }
                        }
                    }
                    ui.separator();
                    if ui.button("Close").clicked() {
                        ui.close();
                        self.close_board();
                    }
                });
                ui.menu_button("Board", |ui| {
                    if ui.button("Set Size…").clicked() {
                        ui.close();
                        self.show_set_size_dialog = true;
                        self.set_size_proto_cols = self.protomatrix_config.proto_area.0;
                        self.set_size_proto_rows = self.protomatrix_config.proto_area.1;
                        self.set_size_matrix_size = self.protomatrix_config.matrix_size;
                        self.set_size_break_every = self.protomatrix_config.matrix_break_every;
                    }
                });
            });
        });

        // Set Size dialog
        if self.show_set_size_dialog {
            egui::Window::new("Board Size")
                .collapsible(false)
                .resizable(false)
                .show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        ui.label("Proto area (cols × rows):");
                        ui.add(egui::DragValue::new(&mut self.set_size_proto_cols).range(1..=200));
                        ui.label("×");
                        ui.add(egui::DragValue::new(&mut self.set_size_proto_rows).range(1..=50));
                    });
                    ui.horizontal(|ui| {
                        ui.label("Matrix size:");
                        ui.add(egui::DragValue::new(&mut self.set_size_matrix_size).range(1..=64));
                    });
                    ui.horizontal(|ui| {
                        ui.label("Break every:");
                        ui.add(egui::DragValue::new(&mut self.set_size_break_every).range(1..=64));
                    });
                    ui.separator();
                    ui.horizontal(|ui| {
                        if ui.button("Apply").clicked() {
                            self.set_size_apply_clicked = true;
                            self.show_set_size_dialog = false;
                        }
                        if ui.button("Cancel").clicked() {
                            self.show_set_size_dialog = false;
                        }
                    });
                });
        }

        // Rename net dialog
        let rename_net_idx = self.show_rename_net_dialog;
        if let Some(net_idx) = rename_net_idx {
            egui::Window::new("Name Net")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
                .show(ctx, |ui| {
                    ui.label(format!("Net {}", net_idx));
                    ui.horizontal(|ui| {
                        ui.label("Name:");
                        ui.text_edit_singleline(&mut self.rename_net_name);
                    });
                    ui.separator();
                    ui.horizontal(|ui| {
                        if ui.button("OK").clicked() {
                            let name = self.rename_net_name.trim().to_string();
                            if name.is_empty() {
                                self.net_names.remove(&net_idx);
                            } else {
                                self.net_names.insert(net_idx, name);
                            }
                            self.show_rename_net_dialog = None;
                        }
                        if ui.button("Cancel").clicked() {
                            self.show_rename_net_dialog = None;
                        }
                    });
                });
        }

        // Process Apply (runs when dialog was closed via Apply)
        if self.set_size_apply_clicked {
            self.set_size_apply_clicked = false;
            self.protomatrix_config.proto_area =
                (self.set_size_proto_cols, self.set_size_proto_rows);
            self.protomatrix_config.matrix_size = self.set_size_matrix_size;
            self.protomatrix_config.matrix_break_every = self.set_size_break_every;
            self.autoroute_error = None;
            match router::autoroute(&self.protomatrix_config, &self.connections) {
                AutorouteResult::Ok(js) => self.jumper_state = Some(js),
                AutorouteResult::Err(e) => {
                    self.autoroute_error = Some(e);
                    self.jumper_state = None;
                }
            }
        }

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.set_min_size(ui.available_size());
            let (x_min, x_max, y_min, y_max) =
                protomatrix::board_bounds_mm(&self.protomatrix_config);
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

            let sense = egui::Sense::click()
                .union(egui::Sense::drag())
                .union(egui::Sense::hover());
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
            let package_visible = self.dip_lib.is_some();
            let pointer_over_package = package_visible
                && pointer_mm.map_or(false, |p| {
                    self.dip_lib.as_ref().map_or(false, |lib| {
                        lib.iter().next().map_or(false, |(_name, fp)| {
                            let bounds = footprint::footprint_bounds_local(fp);
                            footprint::point_in_footprint_bounds(
                                p.x, p.y,
                                dip_pos_mm.x, dip_pos_mm.y,
                                self.dip_rotation_deg as f64,
                                bounds,
                            )
                        })
                    })
                });

            // Start connection drag when pressing on a pad (and not over package)
            if ui.input(|i| i.pointer.primary_pressed()) && !self.dragging_package {
                if let Some(target) = pointer_mm.and_then(|p| protomatrix::hit_test(&self.protomatrix_config, p.x, p.y)) {
                    if matches!(target, ProtomatrixTarget::Pad { .. }) && !pointer_over_package {
                        self.connection_drag_source = Some(target);
                    }
                }
            }
            // Complete or cancel connection drag on release
            if ui.input(|i| i.pointer.primary_released()) {
                if let Some(source) = self.connection_drag_source.take() {
                    if let Some(dest) = pointer_mm.and_then(|p| protomatrix::hit_test(&self.protomatrix_config, p.x, p.y)) {
                        if matches!(dest, ProtomatrixTarget::Pad { .. }) && dest != source {
                            self.connections.push(Connection::new(source, dest));
                            self.autoroute_error = None;
                            match router::autoroute(&self.protomatrix_config, &self.connections) {
                                AutorouteResult::Ok(js) => self.jumper_state = Some(js),
                                AutorouteResult::Err(e) => {
                                    self.autoroute_error = Some(e);
                                    self.jumper_state = None;
                                }
                            }
                        }
                    }
                }
            }

            if response.dragged_by(egui::PointerButton::Primary) {
                let delta = response.drag_delta();
                let delta_mm = egui::vec2(delta.x / scale, delta.y / scale);
                if (self.dragging_package || pointer_over_package) && package_visible
                    && self.connection_drag_source.is_none()
                {
                    if !self.dragging_package {
                        self.dip_pos_mm = Some(dip_pos_mm);
                        self.dragging_package = true;
                    }
                    if let Some(ref mut pos) = self.dip_pos_mm {
                        *pos += delta_mm;
                    }
                }
            }

            // Protomatrix hover/click events (only when not dragging package)
            let primary_clicked = response.clicked() && !self.dragging_package;
            let primary_double_clicked = response.double_clicked() && !self.dragging_package;
            protomatrix::handle_pointer_input(
                &self.protomatrix_config,
                pointer_mm,
                primary_clicked,
                &mut self.protomatrix_state,
            );
            // Right-click context menu: Disconnect column from net
            let matrix_size = self.protomatrix_config.matrix_size;
            let context_col = pointer_mm
                .and_then(|p| protomatrix::hit_test(&self.protomatrix_config, p.x, p.y))
                .and_then(|t| column_from_target(&t));
            let context_col_with_net = context_col.and_then(|col| {
                self.jumper_state
                    .as_ref()
                    .and_then(|js| js.net_for_column(col.side, col.col, matrix_size))
                    .map(|_| col)
            });
            // Capture column when menu opens (pointer moves to menu, so we must store it)
            if response.secondary_clicked() {
                self.context_menu_column = context_col_with_net;
            } else if !response.context_menu_opened() {
                // Only clear when menu closed (not when we just opened it this frame)
                self.context_menu_column = None;
            }
            let disconnect_id = egui::Id::new("protomatrix_disconnect_column");
            response.context_menu(|ui| {
                if let Some(col) = self.context_menu_column {
                    if ui.button("Disconnect").clicked() {
                        ui.data_mut(|d| d.insert_temp(disconnect_id, Some(col)));
                        ui.close();
                    }
                    ui.separator();
                    ui.menu_button("Change color", |ui| {
                        let matrix_size = self.protomatrix_config.matrix_size;
                        let net_idx = self
                            .jumper_state
                            .as_ref()
                            .and_then(|js| js.net_for_column(col.side, col.col, matrix_size));
                        if let (Some(net_idx), Some(js)) = (net_idx, self.jumper_state.as_ref()) {
                            if let Some(canonical_key) = js.net_canonical_keys.get(net_idx) {
                                let key_str = column_key_to_string(canonical_key);
                                let default_color =
                                    protomatrix::net_color(net_idx);
                                let mut color = self
                                    .net_colors
                                    .get(&key_str)
                                    .map(|rgba| {
                                        egui::Color32::from_rgba_unmultiplied(
                                            rgba[0], rgba[1], rgba[2], rgba[3],
                                        )
                                    })
                                    .unwrap_or(default_color);
                                if egui::widgets::color_picker::color_picker_color32(
                                    ui,
                                    &mut color,
                                    egui::widgets::color_picker::Alpha::Opaque,
                                ) {
                                    let rgba = color.to_srgba_unmultiplied();
                                    self.net_colors
                                        .insert(key_str.clone(), [rgba[0], rgba[1], rgba[2], rgba[3]]);
                                }
                                if ui.button("Reset to default").clicked() {
                                    self.net_colors.remove(&key_str);
                                    ui.close();
                                }
                            }
                        }
                    });
                } else {
                    ui.label("(Right-click on pad or jumper to disconnect)");
                }
            });

            if let Some(ref t) = self.protomatrix_state.clicked {
                let matrix_size = self.protomatrix_config.matrix_size;
                let net_idx = net_index_for_target(
                    t,
                    self.jumper_state.as_ref(),
                    matrix_size,
                );
                self.selected_net = net_idx;

                if primary_double_clicked {
                    // Double-click: open rename dialog for the net
                    if let Some(ni) = net_idx {
                        self.show_rename_net_dialog = Some(ni);
                        self.rename_net_name = self
                            .net_names
                            .get(&ni)
                            .cloned()
                            .unwrap_or_else(|| format!("Net {}", ni));
                    }
                } else {
                    // Single click: selection only (connections use drag)
                    self.last_clicked = Some(t.clone());
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
            protomatrix::draw_tracks(&self.protomatrix_config, &painter, &to_screen, scale, view);
            protomatrix::draw_proto_pads(&self.protomatrix_config, &painter, &to_screen, scale, view);
            let net_color_override = self
                .jumper_state
                .as_ref()
                .filter(|_| !self.net_colors.is_empty())
                .map(|js| {
                    let net_colors = self.net_colors.clone();
                    let keys = js.net_canonical_keys.clone();
                    let closure = move |net_idx: usize| -> egui::Color32 {
                        keys.get(net_idx)
                            .and_then(|ck| net_colors.get(&column_key_to_string(ck)))
                            .map(|rgba| {
                                egui::Color32::from_rgba_unmultiplied(
                                    rgba[0], rgba[1], rgba[2], rgba[3],
                                )
                            })
                            .unwrap_or_else(|| protomatrix::net_color(net_idx))
                    };
                    closure
                });
            let color_override_ref = net_color_override
                .as_ref()
                .map(|c| c as &dyn Fn(usize) -> egui::Color32);
            protomatrix::draw_proto_jumper_indicators(
                &self.protomatrix_config,
                &painter,
                &to_screen,
                scale,
                view,
                self.jumper_state.as_ref(),
                color_override_ref,
            );
            protomatrix::draw_matrix_areas(
                &self.protomatrix_config,
                &painter,
                &to_screen,
                scale,
                view,
                self.jumper_state.as_ref(),
                color_override_ref,
            );
            protomatrix::draw_y_link_vias(
                &self.protomatrix_config,
                &painter,
                &to_screen,
                scale,
                view,
            );
            protomatrix::draw_mounting_holes(
                &self.protomatrix_config,
                &painter,
                &to_screen,
                scale,
                view,
            );
            protomatrix::draw_silkscreen_proto_column_labels(
                &self.protomatrix_config,
                &painter,
                &to_screen,
                scale,
                view,
            );
            protomatrix::draw_silkscreen_x_labels(
                &self.protomatrix_config,
                &painter,
                &to_screen,
                scale,
                view,
            );
            protomatrix::draw_silkscreen_y_labels(
                &self.protomatrix_config,
                &painter,
                &to_screen,
                scale,
                view,
            );

            // Draw grey drag line while creating a connection (hidden when released)
            if let (Some(ref source), Some(ptr)) = (&self.connection_drag_source, pointer_mm) {
                protomatrix::draw_connection_drag_line(
                    &self.protomatrix_config,
                    &painter,
                    &to_screen,
                    scale,
                    source,
                    ptr,
                );
            }

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
            } else if self.protomatrix_state.hovered.is_some() {
                ctx.set_cursor_icon(egui::CursorIcon::PointingHand);
            } else if response.hovered() {
                ctx.set_cursor_icon(egui::CursorIcon::Grab);
            }

            // Process Disconnect from context menu (remove_temp consumes so we only process once)
            if let Some(col) = ctx.data_mut(|d| d.remove_temp::<Option<ColumnKey>>(disconnect_id)).flatten() {
                disconnect_column(&mut self.connections, col);
                self.connection_drag_source = None;
                self.selected_net = None;
                self.context_menu_column = None;
                self.autoroute_error = None;
                match router::autoroute(&self.protomatrix_config, &self.connections) {
                    AutorouteResult::Ok(js) => self.jumper_state = Some(js),
                    AutorouteResult::Err(e) => {
                        self.autoroute_error = Some(e);
                        self.jumper_state = None;
                    }
                }
            }
        });

        // Status bar: show hover, last clicked, selected net, connections, and Clear button
        egui::TopBottomPanel::bottom("protomatrix_status").show(ctx, |ui| {
            let hover_txt = self
                .protomatrix_state
                .hovered
                .as_ref()
                .map(protomatrix_target_label)
                .unwrap_or_else(|| "—".to_string());
            let click_txt = self
                .last_clicked
                .as_ref()
                .map(protomatrix_target_label)
                .unwrap_or_else(|| "—".to_string());
            let net_txt = self
                .selected_net
                .map(|ni| {
                    self.net_names
                        .get(&ni)
                        .cloned()
                        .unwrap_or_else(|| format!("Net {}", ni))
                })
                .unwrap_or_else(|| "—".to_string());
            ui.horizontal(|ui| {
                ui.label("Hover:");
                ui.monospace(hover_txt);
                ui.separator();
                ui.label("Last click:");
                ui.monospace(click_txt);
                ui.separator();
                ui.label("Selected net:");
                ui.monospace(net_txt);
                if self.selected_net.is_some() {
                    ui.weak("(double-click to rename)");
                }
                ui.separator();
                ui.label(format!(
                    "Connections: {}",
                    self.connections.len()
                ));
                if self.connection_drag_source.is_some() {
                    ui.label("(drag to pad)");
                }
                if let Some(ref err) = self.autoroute_error {
                    ui.colored_label(egui::Color32::RED, err);
                } else if let Some(ref js) = self.jumper_state {
                    ui.label(format!("| Jumpers: {} closed", js.closed_count()));
                }
                if ui.button("Clear connections").clicked() {
                    self.connections.clear();
                    self.connection_drag_source = None;
                    self.jumper_state = None;
                    self.autoroute_error = None;
                    self.selected_net = None;
                }
            });
        });
    }
}
