use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;

use eframe::egui;
use serde::{Deserialize, Serialize};

#[cfg(target_arch = "wasm32")]
use eframe::wasm_bindgen::{closure::Closure, JsCast};

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

/// Display label for dialogs (e.g. "XA9" for Upper col 8, "XB1" for Lower col 0). Matches silkscreen X labels.
pub fn column_key_to_x_label(col: &ColumnKey) -> String {
    let side = match col.side {
        ProtoSide::Lower => 'B',
        ProtoSide::Upper => 'A',
    };
    format!("X{}{}", side, col.col + 1)
}

/// Board file format for save/load.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BoardFile {
    pub config: ProtomatrixConfig,
    pub connections: Vec<Connection>,
    /// Net names keyed by canonical column key (e.g. "L3", "U5") for stability when adding connections.
    #[serde(default)]
    pub net_names: HashMap<String, String>,
    /// Net color overrides keyed by canonical column key (e.g. "L3", "U5").
    #[serde(default)]
    pub net_colors: HashMap<String, [u8; 4]>,
    /// Net row pin overrides: canonical key -> Y row index. Overrides autorouter assignment.
    #[serde(default)]
    pub net_row_pins: HashMap<String, i32>,
    /// Free-form text annotations placed on the canvas (position mm, text).
    #[serde(default)]
    pub annotations: Vec<Annotation>,
    /// Outer row column annotations (same position as net labels), keyed by column key (e.g. "L3", "U5").
    #[serde(default)]
    pub column_annotations: HashMap<String, String>,
}

/// Free-form text annotation at a position on the canvas.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Annotation {
    pub pos_mm: [f32; 2],
    pub text: String,
}

/// State when editing an annotation inline.
enum EditingAnnotation {
    /// Creating new annotation (index in annotations).
    New { index: usize },
    /// Editing existing annotation by index.
    Existing { index: usize, original: String },
}

mod protomatrix;
mod router;

#[cfg(target_arch = "wasm32")]
thread_local! {
    static PENDING_FILE_LOAD: RefCell<Option<String>> = RefCell::new(None);
}

/// Trigger native file picker and read selected file as text. On completion, stores result in
/// PENDING_FILE_LOAD and requests repaint. Call ctx.request_repaint() after this to process next frame.
#[cfg(target_arch = "wasm32")]
fn wasm_pick_file_for_open(ctx: egui::Context) {
    let document = web_sys::window()
        .and_then(|w| w.document())
        .expect("No document");
    let input = document
        .create_element("input")
        .expect("create input")
        .dyn_into::<web_sys::HtmlInputElement>()
        .expect("input element");
    input.set_attribute("type", "file").expect("set type");
    input.set_attribute("accept", ".json,application/json").expect("set accept");
    input.style().set_property("display", "none").expect("set style");
    document.body().expect("no body").append_child(&input).expect("append");

    let ctx = ctx.clone();
    let on_change = Closure::once(Box::new(move |e: web_sys::Event| {
        let target = e.target().and_then(|t| t.dyn_into::<web_sys::HtmlInputElement>().ok());
        if let Some(input) = target {
            if let Some(files) = input.files() {
                if let Some(file) = files.get(0) {
                    let reader = web_sys::FileReader::new().expect("FileReader");
                    let reader_clone = reader.clone();
                    let ctx = ctx.clone();
                    let on_load = Closure::once(Box::new(move |_: web_sys::ProgressEvent| {
                        if let Ok(result) = reader_clone.result() {
                            if let Some(text) = result.as_string() {
                                PENDING_FILE_LOAD.with(|cell| *cell.borrow_mut() = Some(text));
                            }
                        }
                        ctx.request_repaint();
                    }));
                    reader.set_onload(Some(on_load.as_ref().unchecked_ref()));
                    let _ = reader.read_as_text(&file);
                    on_load.forget();
                }
            }
            if let Some(parent) = input.parent_element() {
                let _ = parent.remove_child(&input);
            }
        }
    }) as Box<dyn FnOnce(_)>);
    input.set_onchange(Some(on_change.as_ref().unchecked_ref()));
    on_change.forget();
    let _ = input.click();
}

/// Trigger download of board JSON. In browser, creates a blob URL and simulates anchor click.
#[cfg(target_arch = "wasm32")]
fn wasm_download_board(filename: &str, content: &str) {
    let window = web_sys::window().expect("No window");
    let document = window.document().expect("No document");
    let opts = web_sys::BlobPropertyBag::new();
    opts.set_type("application/json");
    let blob = web_sys::Blob::new_with_str_sequence_and_options(
        &js_sys::Array::of1(&js_sys::JsString::from(content)),
        &opts,
    )
    .expect("Blob");
    let url = web_sys::Url::create_object_url_with_blob(&blob).expect("Object URL");
    let a = document
        .create_element("a")
        .expect("create anchor")
        .dyn_into::<web_sys::HtmlAnchorElement>()
        .expect("anchor");
    a.set_href(&url);
    a.set_download(filename);
    a.style().set_property("display", "none").expect("set style");
    document.body().expect("no body").append_child(&a).expect("append");
    a.click();
    let _ = document.body().unwrap().remove_child(&a);
    let _ = web_sys::Url::revoke_object_url(&url);
}

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

/// Migrate net names, colors, and row pins to current canonical keys. Handles:
/// 1. Old index-based keys ("0", "1") in net_names from legacy files.
/// 2. Stale canonical keys: when a net extends to include a smaller column, its canonical key
///    changes (e.g. L2→L1). Migrate names, colors, and row pins from the old key to the new one.
/// Returns true if any row pins were migrated (moved from a column key to canonical).
/// Caller should re-run autoroute when this happens, since the first autoroute ran
/// before the migration and may have ignored pins whose canonical changed (e.g. after merge).
fn migrate_net_metadata_after_autoroute(
    net_names: &mut HashMap<String, String>,
    net_colors: &mut HashMap<String, [u8; 4]>,
    net_row_pins: &mut HashMap<String, i32>,
    js: &JumperState,
    matrix_size: i32,
) -> bool {
    // 1. Migrate index-based keys in net_names (backward compat)
    let index_keys: Vec<String> = net_names
        .keys()
        .filter(|k| k.parse::<usize>().is_ok())
        .cloned()
        .collect();
    for key in index_keys {
        if let Ok(idx) = key.parse::<usize>() {
            if let Some(ck) = js.net_canonical_keys.get(idx) {
                if let Some(name) = net_names.remove(&key) {
                    net_names.insert(column_key_to_string(ck), name);
                }
            }
        }
    }
    // 2. Migrate stale canonical keys for both names and colors
    let mut pins_migrated = false;
    for (net_idx, &canonical) in js.net_canonical_keys.iter().enumerate() {
        let canon_str = column_key_to_string(&canonical);
        let columns = js.columns_for_net(net_idx);
        // Migrate name if canonical has none
        if !net_names.contains_key(&canon_str) {
            for col in &columns {
                let k = column_key_to_string(col);
                if let Some(name) = net_names.remove(&k) {
                    net_names.insert(canon_str.clone(), name);
                    break;
                }
            }
        }
        // Migrate color if canonical has none
        if !net_colors.contains_key(&canon_str) {
            for col in &columns {
                let k = column_key_to_string(col);
                if let Some(rgba) = net_colors.remove(&k) {
                    net_colors.insert(canon_str.clone(), rgba);
                    break;
                }
            }
        }
        // Migrate row pin if canonical has none; clamp to valid range
        if !net_row_pins.contains_key(&canon_str) {
            for col in &columns {
                let k = column_key_to_string(col);
                if let Some(row) = net_row_pins.remove(&k) {
                    let clamped = row.clamp(0, matrix_size - 1);
                    net_row_pins.insert(canon_str.clone(), clamped);
                    pins_migrated = true; // Caller must re-run autoroute to apply the migrated pin
                    break;
                }
            }
        }
    }
    // Remove orphaned pins: keys that are not the canonical of any net. When upper/lower nets
    // merge, both old canonicals (e.g. L1 and U3) may have pins; only the new canonical (L1)
    // is used for assignment. The U3 pin is orphaned and can cause "moved nets remain in place"
    // when it conflicts with a later row drag.
    let canonical_keys: std::collections::HashSet<String> = js
        .net_canonical_keys
        .iter()
        .map(|ck| column_key_to_string(ck))
        .collect();
    net_row_pins.retain(|k, _| canonical_keys.contains(k));
    // Clamp any pins that are out of range after migration
    let valid_range = 0..matrix_size;
    net_row_pins.retain(|_, row| valid_range.contains(row));
    pins_migrated
}

#[cfg(not(target_arch = "wasm32"))]
fn main() -> eframe::Result<()> {
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

#[cfg(target_arch = "wasm32")]
fn main() {
    use eframe::wasm_bindgen::JsCast;

    eframe::WebLogger::init(log::LevelFilter::Debug).ok();

    let web_options = eframe::WebOptions::default();

    wasm_bindgen_futures::spawn_local(async {
        let document = web_sys::window()
            .expect("No window")
            .document()
            .expect("No document");

        let canvas = document
            .get_element_by_id("the_canvas_id")
            .expect("Failed to find the_canvas_id")
            .dyn_into::<web_sys::HtmlCanvasElement>()
            .expect("the_canvas_id was not an HtmlCanvasElement");

        let start_result = eframe::WebRunner::new()
            .start(
                canvas,
                web_options,
                Box::new(|cc| Ok(Box::new(CanvasApp::new(cc)))),
            )
            .await;

        if let Some(loading_text) = document.get_element_by_id("loading_text") {
            match start_result {
                Ok(_) => {
                    loading_text.remove();
                }
                Err(e) => {
                    loading_text.set_inner_html(
                        "The app has crashed. See the developer console for details.",
                    );
                    panic!("Failed to start eframe: {e:?}");
                }
            }
        }
    });
}

const ANNOTATION_FONT_SIZE: f32 = 12.0;
const ZOOM_MIN: f32 = 0.15;
const ZOOM_MAX: f32 = 25.0;
/// Scroll-to-zoom: scale factor per point of smooth scroll (reduces stutter vs raw_scroll_delta).
const ZOOM_SENSITIVITY: f32 = 0.0008;

struct CanvasApp {
    pan: egui::Vec2,
    zoom: f32,
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
    /// User-assigned names for nets keyed by canonical column key (e.g. "L3", "U5").
    net_names: HashMap<String, String>,
    /// Net color overrides keyed by canonical column key (e.g. "L3", "U5").
    net_colors: HashMap<String, [u8; 4]>,
    /// Net row pin overrides: canonical key -> Y row index. Overrides autorouter assignment.
    net_row_pins: HashMap<String, i32>,
    /// Row drag: (source_row, net_idx) when dragging a Y row to swap nets.
    row_drag_source: Option<(i32, usize)>,
    /// Net index of the selected element (pad or jumper with a net).
    selected_net: Option<usize>,
    /// When Some(canonical_key), show rename dialog for that net.
    show_rename_net_dialog: Option<String>,
    /// Buffer for the rename net dialog.
    rename_net_name: String,
    /// Column for context menu (captured when menu opens; any column for Annotate).
    context_menu_column: Option<ColumnKey>,
    /// Column with net (captured when menu opens; for Disconnect/Change color). Stays stable to avoid flicker.
    context_menu_column_with_net: Option<ColumnKey>,
    /// Floating annotation index for context menu (captured when right-click on annotation).
    context_menu_annotation: Option<usize>,
    /// When Some(col), show Annotate dialog for that column.
    show_annotate_dialog: Option<ColumnKey>,
    /// Buffer for the annotate column dialog.
    annotate_column_text: String,
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
    /// Free-form text annotations on the canvas.
    annotations: Vec<Annotation>,
    /// Outer row column annotations (same position as net labels above/below columns).
    column_annotations: HashMap<String, String>,
    /// When Some, user is editing an annotation inline (TextEdit overlay).
    editing_annotation: Option<EditingAnnotation>,
    /// When Some(index), user is dragging this annotation to move it.
    annotation_drag_source: Option<usize>,
}

impl CanvasApp {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        // Add Rockwell font for PCB silkscreen labels
        let mut fonts = egui::FontDefinitions::default();
        fonts.font_data.insert(
            "Rockwell".to_owned(),
            std::sync::Arc::new(egui::FontData::from_static(
                include_bytes!("../fonts/Rockwell-Regular.ttf"),
            )),
        );
        fonts.font_data.insert(
            "Rockwell Bold".to_owned(),
            std::sync::Arc::new(egui::FontData::from_static(
                include_bytes!("../fonts/Rockwell-Bold.otf"),
            )),
        );
        fonts
            .families
            .entry(egui::FontFamily::Name("Rockwell".into()))
            .or_default()
            .extend(["Rockwell".to_owned(), "Rockwell Bold".to_owned()]);
        fonts
            .families
            .entry(egui::FontFamily::Name("Rockwell Bold".into()))
            .or_default()
            .push("Rockwell Bold".to_owned());
        cc.egui_ctx.set_fonts(fonts);

        Self {
            pan: egui::Vec2::ZERO,
            zoom: 1.0,
            protomatrix_config: ProtomatrixConfig::default(),
            protomatrix_state: ProtomatrixPointerState::default(),
            last_clicked: None,
            connections: Vec::new(),
            connection_drag_source: None,
            jumper_state: None,
            autoroute_error: None,
            net_names: HashMap::new(),
            net_colors: HashMap::new(),
            net_row_pins: HashMap::new(),
            row_drag_source: None,
            selected_net: None,
            show_rename_net_dialog: None,
            rename_net_name: String::new(),
            context_menu_column: None,
            context_menu_column_with_net: None,
            context_menu_annotation: None,
            show_annotate_dialog: None,
            annotate_column_text: String::new(),
            file_path: None,
            show_set_size_dialog: false,
            set_size_proto_cols: 63,
            set_size_proto_rows: 5,
            set_size_matrix_size: 15,
            set_size_break_every: 5,
            set_size_apply_clicked: false,
            annotations: Vec::new(),
            column_annotations: HashMap::new(),
            editing_annotation: None,
            annotation_drag_source: None,
        }
    }

    /// Run autoroute, migrate metadata, and re-run if pins were migrated (canonical changed after merge).
    fn run_autoroute(&mut self) {
        self.autoroute_error = None;
        let matrix_size = self.protomatrix_config.matrix_size;
        let mut result = router::autoroute(
            &self.protomatrix_config,
            &self.connections,
            Some(&self.net_row_pins),
        );
        if let AutorouteResult::Ok(ref js) = result {
            if migrate_net_metadata_after_autoroute(
                &mut self.net_names,
                &mut self.net_colors,
                &mut self.net_row_pins,
                js,
                matrix_size,
            ) {
                result = router::autoroute(
                    &self.protomatrix_config,
                    &self.connections,
                    Some(&self.net_row_pins),
                );
                if let AutorouteResult::Ok(ref js2) = result {
                    migrate_net_metadata_after_autoroute(
                        &mut self.net_names,
                        &mut self.net_colors,
                        &mut self.net_row_pins,
                        js2,
                        matrix_size,
                    );
                }
            }
        }
        match result {
            AutorouteResult::Ok(js) => self.jumper_state = Some(js),
            AutorouteResult::Err(e) => {
                self.autoroute_error = Some(e);
                self.jumper_state = None;
            }
        }
    }

    /// Load board from JSON string. Used by both native (after file read) and WASM (after file picker).
    fn load_board_from_json(&mut self, s: &str) -> Result<(), String> {
        let board: BoardFile = serde_json::from_str(s)
            .map_err(|e| format!("Parse failed: {}", e))?;
        self.protomatrix_config = board.config;
        self.connections = board.connections;
        self.net_names = board.net_names;
        self.net_colors = board.net_colors;
        self.net_row_pins = board.net_row_pins;
        self.annotations = board.annotations;
        self.column_annotations = board.column_annotations;
        self.editing_annotation = None;
        self.connection_drag_source = None;
        self.row_drag_source = None;
        self.selected_net = None;
        self.run_autoroute();
        Ok(())
    }

    /// Serialize board to JSON string. Used by both native (before file write) and WASM (for download).
    fn save_board_json(&self) -> Result<String, String> {
        let board = BoardFile {
            config: self.protomatrix_config.clone(),
            connections: self.connections.clone(),
            net_names: self.net_names.clone(),
            net_colors: self.net_colors.clone(),
            net_row_pins: self.net_row_pins.clone(),
            annotations: self.annotations.clone(),
            column_annotations: self.column_annotations.clone(),
        };
        serde_json::to_string_pretty(&board).map_err(|e| format!("Serialize failed: {}", e))
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn load_board(&mut self, path: &std::path::Path) -> Result<(), String> {
        let s = std::fs::read_to_string(path)
            .map_err(|e| format!("Read failed: {}", e))?;
        self.load_board_from_json(&s)?;
        self.file_path = Some(path.to_path_buf());
        Ok(())
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn save_board(&self, path: &std::path::Path) -> Result<(), String> {
        let s = self.save_board_json()?;
        std::fs::write(path, s).map_err(|e| format!("Write failed: {}", e))?;
        Ok(())
    }

    /// Return index of annotation under pointer_mm, or None. Hit-tests in reverse order (top-most first).
    fn annotation_index_at(&self, pointer_mm: egui::Vec2, scale: f32, ui: &egui::Ui) -> Option<usize> {
        let font_id = egui::FontId::proportional(ANNOTATION_FONT_SIZE);
        let editing_index = self.editing_annotation.as_ref().map(|e| match e {
            EditingAnnotation::New { index } | EditingAnnotation::Existing { index, .. } => *index,
        });
        for i in (0..self.annotations.len()).rev() {
            if editing_index == Some(i) {
                continue;
            }
            let ann = &self.annotations[i];
            let text = if ann.text.is_empty() { " " } else { ann.text.as_str() };
            let galley = ui.fonts_mut(|f| {
                f.layout_no_wrap(
                    text.to_string(),
                    font_id.clone(),
                    egui::Color32::WHITE,
                )
            });
            let w_mm = galley.size().x / scale;
            let h_mm = galley.size().y / scale;
            let px = ann.pos_mm[0];
            let py = ann.pos_mm[1];
            if pointer_mm.x >= px - w_mm / 2.0
                && pointer_mm.x <= px + w_mm / 2.0
                && pointer_mm.y >= py
                && pointer_mm.y <= py + h_mm
            {
                return Some(i);
            }
        }
        None
    }

    /// Handle double-click for annotation create/edit. Hit-tests annotations (reverse order);
    /// if over one, enter edit mode; else create new at pointer_mm.
    fn handle_annotation_double_click(&mut self, pointer_mm: egui::Vec2, scale: f32, ui: &egui::Ui) {
        if let Some(i) = self.annotation_index_at(pointer_mm, scale, ui) {
            let original = self.annotations[i].text.clone();
            self.editing_annotation = Some(EditingAnnotation::Existing {
                index: i,
                original,
            });
            return;
        }
        // Not over any annotation: create new
        self.annotations.push(Annotation {
            pos_mm: [pointer_mm.x, pointer_mm.y],
            text: String::new(),
        });
        let idx = self.annotations.len() - 1;
        self.editing_annotation = Some(EditingAnnotation::New { index: idx });
    }

    fn close_board(&mut self) {
        self.protomatrix_config = ProtomatrixConfig::default();
        self.connections.clear();
        self.net_names.clear();
        self.net_colors.clear();
        self.net_row_pins.clear();
        self.annotations.clear();
        self.column_annotations.clear();
        self.editing_annotation = None;
        self.annotation_drag_source = None;
        self.context_menu_column = None;
        self.context_menu_annotation = None;
        self.row_drag_source = None;
        self.connection_drag_source = None;
        self.jumper_state = None;
        self.autoroute_error = None;
        self.file_path = None;
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

/// When disconnecting a column, migrate its metadata (name, color, row pin) to the remaining net's
/// canonical key. Otherwise the disconnected column's key becomes orphaned and the remaining net
/// loses its color/name.
fn migrate_disconnected_column_metadata(
    col: ColumnKey,
    js: &JumperState,
    matrix_size: i32,
    net_names: &mut HashMap<String, String>,
    net_colors: &mut HashMap<String, [u8; 4]>,
    net_row_pins: &mut HashMap<String, i32>,
) {
    let Some(net_idx) = js.net_for_column(col.side, col.col, matrix_size) else {
        return;
    };
    let columns = js.columns_for_net(net_idx);
    let remaining: Vec<ColumnKey> = columns.into_iter().filter(|c| *c != col).collect();
    let Some(&new_canonical) = remaining.iter().min() else {
        return; // col was the only column, nothing to migrate to
    };
    let col_key = column_key_to_string(&col);
    let new_key = column_key_to_string(&new_canonical);

    if !net_names.contains_key(&new_key) {
        if let Some(name) = net_names.remove(&col_key) {
            net_names.insert(new_key.clone(), name);
        }
    }
    if !net_colors.contains_key(&new_key) {
        if let Some(rgba) = net_colors.remove(&col_key) {
            net_colors.insert(new_key.clone(), rgba);
        }
    }
    if !net_row_pins.contains_key(&new_key) {
        if let Some(row) = net_row_pins.remove(&col_key) {
            let clamped = row.clamp(0, matrix_size - 1);
            net_row_pins.insert(new_key.clone(), clamped);
        }
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

/// Get net index for a target (pad, solder jumper, or matrix row) if it belongs to a net.
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
        ProtomatrixTarget::MatrixRow { side: _, row } => js.net_for_row(*row),
    }
}

fn row_to_y_label(row: i32) -> String {
    format!("Y{}", row + 1)
}

fn protomatrix_target_label(t: &ProtomatrixTarget) -> String {
    let side = |s: ProtoSide| match s {
        ProtoSide::Upper => "upper",
        ProtoSide::Lower => "lower",
    };
    match t {
        ProtomatrixTarget::Pad { side: s, col, row } => {
            format!("pad {} ({})", column_key_to_x_label(&ColumnKey { side: *s, col: *col }), row_to_y_label(*row))
        }
        ProtomatrixTarget::MatrixRow { side: s, row } => {
            format!("matrix {} [{}]", row_to_y_label(*row), side(*s))
        }
        ProtomatrixTarget::SolderJumper { side: s, col, row } => {
            format!("jumper {} {} [{}]", column_key_to_x_label(&ColumnKey { side: *s, col: *col }), row_to_y_label(*row), side(*s))
        }
    }
}

impl eframe::App for CanvasApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Process pending file load from WASM file picker
        #[cfg(target_arch = "wasm32")]
        {
            if let Some(content) = PENDING_FILE_LOAD.with(|c| c.borrow_mut().take()) {
                if let Err(e) = self.load_board_from_json(&content) {
                    log::error!("Load error: {}", e);
                }
            }
        }

        // Menu bar
        egui::TopBottomPanel::top("menu_bar").show(ctx, |ui| {
            egui::MenuBar::new().ui(ui, |ui| {
                ui.menu_button("File", |ui| {
                    #[cfg(not(target_arch = "wasm32"))]
                    {
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
                    }
                    #[cfg(target_arch = "wasm32")]
                    {
                        if ui.button("Open…").clicked() {
                            ui.close();
                            wasm_pick_file_for_open(ctx.clone());
                        }
                        if ui.button("Save").clicked() {
                            ui.close();
                            if let Ok(json) = self.save_board_json() {
                                wasm_download_board("board.json", &json);
                            }
                        }
                        if ui.button("Save As…").clicked() {
                            ui.close();
                            if let Ok(json) = self.save_board_json() {
                                wasm_download_board("board.json", &json);
                            }
                        }
                        ui.separator();
                    }
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

        // Rename net dialog (Edit NET)
        let rename_net_key = self.show_rename_net_dialog.clone();
        if let Some(ref key) = rename_net_key {
            let columns_label = self.jumper_state.as_ref().and_then(|js| {
                let net_idx = js.net_canonical_keys.iter().position(|ck| column_key_to_string(ck) == *key)?;
                let cols: Vec<String> = js
                    .columns_for_net(net_idx)
                    .into_iter()
                    .map(|c| column_key_to_x_label(&c))
                    .collect();
                let mut cols = cols;
                cols.sort();
                Some(cols.join(", "))
            });
            let key_x_label = (|| {
                let side = if key.starts_with('L') {
                    ProtoSide::Lower
                } else if key.starts_with('U') {
                    ProtoSide::Upper
                } else {
                    return None;
                };
                let col: i32 = key[1..].parse().ok()?;
                Some(column_key_to_x_label(&ColumnKey { side, col }))
            })();
            egui::Window::new("Edit NET")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
                .show(ctx, |ui| {
                    if let Some(ref cols) = columns_label {
                        ui.label(format!("Columns: {}", cols));
                    } else if let Some(ref x) = key_x_label {
                        ui.label(format!("Net {}", x));
                    } else {
                        ui.label(format!("Net {}", key));
                    }
                    ui.horizontal(|ui| {
                        ui.label("Name:");
                        ui.text_edit_singleline(&mut self.rename_net_name);
                    });
                    ui.separator();
                    ui.horizontal(|ui| {
                        if ui.button("OK").clicked() {
                            let name = self.rename_net_name.trim().to_string();
                            if name.is_empty() {
                                self.net_names.remove(key);
                            } else {
                             
                                self.net_names.insert(key.clone(), name);
                            }
                            self.show_rename_net_dialog = None;
                        }
                        if ui.button("Cancel").clicked() {
                            self.show_rename_net_dialog = None;
                        }
                    });
                });
        }

        // Annotate column dialog (outer row label)
        let annotate_col = self.show_annotate_dialog.clone();
        if let Some(col) = annotate_col {
            let key = column_key_to_string(&col);
            let x_label = column_key_to_x_label(&col);
            egui::Window::new("Annotate Column")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
                .show(ctx, |ui| {
                    ui.label(format!("Column {}", x_label));
                    ui.horizontal(|ui| {
                        ui.label("Text:");
                        ui.text_edit_singleline(&mut self.annotate_column_text);
                    });
                    ui.separator();
                    ui.horizontal(|ui| {
                        if ui.button("OK").clicked() {
                            let text = self.annotate_column_text.trim().to_string();
                            if text.is_empty() {
                                self.column_annotations.remove(&key);
                            } else {
                                self.column_annotations.insert(key.clone(), text);
                            }
                            self.show_annotate_dialog = None;
                        }
                        if ui.button("Cancel").clicked() {
                            self.show_annotate_dialog = None;
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
            self.run_autoroute();
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

            // Pointer position in world mm
            let pointer_mm = response.hover_pos().map(|pos| {
                egui::vec2(
                    (pos.x - origin.x - self.pan.x) / scale,
                    (pos.y - origin.y - self.pan.y) / scale,
                )
            });

            // Start connection drag when pressing on a pad
            if ui.input(|i| i.pointer.primary_pressed()) {
                let mut started_drag = false;
                if let Some(target) = pointer_mm.and_then(|p| protomatrix::hit_test(&self.protomatrix_config, p.x, p.y)) {
                    if matches!(target, ProtomatrixTarget::Pad { .. }) {
                        self.connection_drag_source = Some(target);
                        started_drag = true;
                    } else {
                        let row = match &target {
                            ProtomatrixTarget::MatrixRow { row, .. } => Some(*row),
                            ProtomatrixTarget::SolderJumper { row, .. } => Some(*row),
                            _ => None,
                        };
                        if let (Some(row), Some(js)) = (row, self.jumper_state.as_ref()) {
                            if let Some(net_idx) = js.net_for_row(row) {
                                self.row_drag_source = Some((row, net_idx));
                                started_drag = true;
                            }
                        }
                    }
                }
                if !started_drag {
                    if let Some(ptr) = pointer_mm {
                        if let Some(idx) = self.annotation_index_at(ptr, scale, ui) {
                            self.annotation_drag_source = Some(idx);
                        }
                    }
                }
            }
            // Complete or cancel connection drag on release
            if ui.input(|i| i.pointer.primary_released()) {
                if let Some((source_row, source_net_idx)) = self.row_drag_source.take() {
                    // Row drag: swap nets if dropped on a different row
                    let target_row = pointer_mm
                        .and_then(|p| protomatrix::hit_test(&self.protomatrix_config, p.x, p.y))
                        .and_then(|t| match &t {
                            ProtomatrixTarget::MatrixRow { row, .. } => Some(*row),
                            ProtomatrixTarget::SolderJumper { row, .. } => Some(*row),
                            _ => None,
                        });
                    if let Some(target_row) = target_row {
                            if target_row != source_row {
                                if let Some(js) = self.jumper_state.as_ref() {
                                    let source_ck = js.net_canonical_keys.get(source_net_idx);
                                    let target_net_idx = js.net_for_row(target_row);
                                    let target_ck = target_net_idx.and_then(|ni| js.net_canonical_keys.get(ni));
                                    if let (Some(&src_ck), Some(&tgt_ck)) = (source_ck, target_ck) {
                                        let src_key = column_key_to_string(&src_ck);
                                        let tgt_key = column_key_to_string(&tgt_ck);
                                        // Clear any other nets' pins to source/target rows to avoid conflicts
                                        self.net_row_pins.retain(|_, r| *r != source_row && *r != target_row);
                                        self.net_row_pins.insert(src_key, target_row);
                                        self.net_row_pins.insert(tgt_key, source_row);
                                    } else if let Some(&src_ck) = source_ck {
                                        // Target row empty: move source net there
                                        let src_key = column_key_to_string(&src_ck);
                                        self.net_row_pins.retain(|_, r| *r != source_row && *r != target_row);
                                        self.net_row_pins.insert(src_key, target_row);
                                    }
                                    self.run_autoroute();
                                }
                            }
                        }
                } else if let Some(source) = self.connection_drag_source.take() {
                    if let Some(dest) = pointer_mm.and_then(|p| protomatrix::hit_test(&self.protomatrix_config, p.x, p.y)) {
                        if matches!(dest, ProtomatrixTarget::Pad { .. }) && dest != source {
                            self.connections.push(Connection::new(source, dest));
                            self.run_autoroute();
                        }
                    }
                } else {
                    self.annotation_drag_source = None;
                }
            }

            if response.dragged_by(egui::PointerButton::Primary) {
                let delta = response.drag_delta();
                let delta_mm = egui::vec2(delta.x / scale, delta.y / scale);
                if let Some(idx) = self.annotation_drag_source {
                    if let Some(ann) = self.annotations.get_mut(idx) {
                        ann.pos_mm[0] += delta_mm.x;
                        ann.pos_mm[1] += delta_mm.y;
                    }
                }
            }

            // Protomatrix hover/click events
            let primary_clicked = response.clicked();
            let primary_double_clicked = response.double_clicked();
            protomatrix::handle_pointer_input(
                &self.protomatrix_config,
                pointer_mm,
                primary_clicked,
                &mut self.protomatrix_state,
            );
            // Right-click context menu: Disconnect column from net, Annotate column
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
            // Capture column or annotation when menu opens; keep stable while menu is open to avoid flicker
            if response.secondary_clicked() {
                let ann_idx = pointer_mm.and_then(|p| self.annotation_index_at(p, scale, ui));
                if let Some(idx) = ann_idx {
                    self.context_menu_annotation = Some(idx);
                    self.context_menu_column = None;
                    self.context_menu_column_with_net = None;
                } else {
                    self.context_menu_annotation = None;
                    self.context_menu_column = context_col;
                    self.context_menu_column_with_net = context_col_with_net;
                }
            } else if !response.context_menu_opened() {
                self.context_menu_column = None;
                self.context_menu_column_with_net = None;
                self.context_menu_annotation = None;
            }
            let disconnect_id = egui::Id::new("protomatrix_disconnect_column");
            let remove_annotation_id = egui::Id::new("remove_floating_annotation");
            response.context_menu(|ui| {
                if let Some(idx) = self.context_menu_annotation {
                    if ui.button("Remove").clicked() {
                        ui.data_mut(|d| d.insert_temp(remove_annotation_id, Some(idx)));
                        ui.close();
                    }
                } else if let Some(col) = self.context_menu_column {
                    if self.context_menu_column_with_net == Some(col) {
                        if ui.button("Disconnect").clicked() {
                            ui.data_mut(|d| d.insert_temp(disconnect_id, Some(col)));
                            ui.close();
                        }
                        if ui.button("Edit NET").clicked() {
                            let matrix_size = self.protomatrix_config.matrix_size;
                            if let (Some(ni), Some(js)) = (
                                self.jumper_state.as_ref().and_then(|js| js.net_for_column(col.side, col.col, matrix_size)),
                                self.jumper_state.as_ref(),
                            ) {
                                if let Some(ck) = js.net_canonical_keys.get(ni) {
                                    let key = column_key_to_string(ck);
                                    self.show_rename_net_dialog = Some(key.clone());
                                    self.rename_net_name = self
                                        .net_names
                                        .get(&key)
                                        .cloned()
                                        .unwrap_or_else(|| format!("Net {}", ni));
                                    ui.close();
                                }
                            }
                        }
                        ui.separator();
                    }
                    if ui.button("Annotate").clicked() {
                        let key = column_key_to_string(&col);
                        self.annotate_column_text = self
                            .column_annotations
                            .get(&key)
                            .cloned()
                            .unwrap_or_default();
                        self.show_annotate_dialog = Some(col);
                        ui.close();
                    }
                    if self.context_menu_column_with_net == Some(col) {
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
                    }
                } else {
                    ui.label("(Right-click on pad or jumper to disconnect or annotate)");
                }
            });

            // Double-click: check annotation first (so free-floating annotations are always editable
            // even when overlapping pads/jumpers). Then column annotation row, then net rename or create new.
            if primary_double_clicked {
                if let Some(ptr) = pointer_mm {
                    if let Some(i) = self.annotation_index_at(ptr, scale, ui) {
                        // Over a floating annotation: edit it (ignore protomatrix target)
                        let original = self.annotations[i].text.clone();
                        self.editing_annotation = Some(EditingAnnotation::Existing {
                            index: i,
                            original,
                        });
                    } else if let Some((side, col)) =
                        protomatrix::column_annotation_at(&self.protomatrix_config, ptr.x, ptr.y)
                    {
                        // Over column annotation row: open annotate dialog
                        let col_key = ColumnKey { side, col };
                        self.annotate_column_text = self
                            .column_annotations
                            .get(&column_key_to_string(&col_key))
                            .cloned()
                            .unwrap_or_default();
                        self.show_annotate_dialog = Some(col_key);
                    } else if let Some(ref t) = self.protomatrix_state.clicked {
                        // Not over annotation: net rename if on net target, else create new
                        let matrix_size = self.protomatrix_config.matrix_size;
                        let net_idx = net_index_for_target(
                            t,
                            self.jumper_state.as_ref(),
                            matrix_size,
                        );
                        let open_net_rename = net_idx.is_some() && self.jumper_state.is_some();
                        if open_net_rename {
                            if let (Some(ni), Some(js)) = (net_idx, self.jumper_state.as_ref()) {
                                if let Some(ck) = js.net_canonical_keys.get(ni) {
                                    let key = column_key_to_string(ck);
                                    self.show_rename_net_dialog = Some(key.clone());
                                    self.rename_net_name = self
                                        .net_names
                                        .get(&key)
                                        .cloned()
                                        .unwrap_or_else(|| format!("Net {}", ni));
                                }
                            }
                        } else {
                            self.handle_annotation_double_click(ptr, scale, ui);
                        }
                    } else {
                        // Empty space: create new annotation
                        self.handle_annotation_double_click(ptr, scale, ui);
                    }
                }
            } else if let Some(ref t) = self.protomatrix_state.clicked {
                let matrix_size = self.protomatrix_config.matrix_size;
                let net_idx = net_index_for_target(
                    t,
                    self.jumper_state.as_ref(),
                    matrix_size,
                );
                self.selected_net = net_idx;
                // Single click: selection only (connections use drag)
                self.last_clicked = Some(t.clone());
            } else if primary_clicked {
                // Clicked on empty space: clear selection
                self.selected_net = None;
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
            protomatrix::draw_tracks(
                &self.protomatrix_config,
                &painter,
                &to_screen,
                scale,
                view,
                self.jumper_state.as_ref(),
                color_override_ref,
                self.selected_net,
            );
            protomatrix::draw_proto_pads(&self.protomatrix_config, &painter, &to_screen, scale, view);
            protomatrix::draw_proto_jumper_indicators(
                &self.protomatrix_config,
                &painter,
                &to_screen,
                scale,
                view,
                self.jumper_state.as_ref(),
                color_override_ref,
                self.selected_net,
            );
            protomatrix::draw_matrix_areas(
                &self.protomatrix_config,
                &painter,
                &to_screen,
                scale,
                view,
                self.jumper_state.as_ref(),
                color_override_ref,
                self.selected_net,
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
            protomatrix::draw_silkscreen_board_size(
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
            let net_label_fn = self.jumper_state.as_ref().map(|js| {
                let net_names = self.net_names.clone();
                let keys = js.net_canonical_keys.clone();
                move |_side: ProtoSide, _col: i32, net_idx: usize| -> String {
                    keys.get(net_idx)
                        .map(|ck| {
                            net_names
                                .get(&column_key_to_string(ck))
                                .cloned()
                                .unwrap_or_else(|| column_key_to_x_label(ck))
                        })
                        .unwrap_or_else(|| format!("Net {}", net_idx))
                }
            });
            protomatrix::draw_net_labels(
                &self.protomatrix_config,
                &painter,
                &to_screen,
                scale,
                view,
                self.jumper_state.as_ref(),
                net_label_fn.as_ref().map(|f| f as &dyn Fn(_, _, _) -> _),
                color_override_ref,
                self.selected_net,
            );
            protomatrix::draw_column_annotations(
                &self.protomatrix_config,
                &painter,
                &to_screen,
                scale,
                view,
                &self.column_annotations,
            );

            // Draw annotations (skip the one being edited)
            let font_id = egui::FontId::proportional(ANNOTATION_FONT_SIZE);
            let editing_index = self.editing_annotation.as_ref().map(|e| match e {
                EditingAnnotation::New { index } | EditingAnnotation::Existing { index, .. } => *index,
            });
            for (i, ann) in self.annotations.iter().enumerate() {
                if editing_index == Some(i) {
                    continue;
                }
                let (px, py) = (ann.pos_mm[0], ann.pos_mm[1]);
                if px >= view.0 && px <= view.1 && py >= view.2 && py <= view.3 {
                    let pos = to_screen(px, py);
                    painter.text(
                        pos,
                        egui::Align2::CENTER_TOP,
                        ann.text.as_str(),
                        font_id.clone(),
                        egui::Color32::WHITE,
                    );
                }
            }

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
            // Draw row drop highlight when dragging a Y row over another row
            if let (Some((source_row, _)), Some(ptr)) =
                (&self.row_drag_source, pointer_mm)
            {
                if let Some(target) =
                    protomatrix::hit_test(&self.protomatrix_config, ptr.x, ptr.y)
                {
                    let target_row = match &target {
                        ProtomatrixTarget::MatrixRow { row, .. } => Some(*row),
                        ProtomatrixTarget::SolderJumper { row, .. } => Some(*row),
                        _ => None,
                    };
                    if let Some(tr) = target_row {
                        if tr != *source_row {
                            protomatrix::draw_row_drop_highlight(
                                &self.protomatrix_config,
                                &painter,
                                &to_screen,
                                scale,
                                view,
                                tr,
                            );
                        }
                    }
                }
            }

            if self.annotation_drag_source.is_some() {
                ctx.set_cursor_icon(egui::CursorIcon::Grabbing);
            } else             if self.annotation_drag_source.is_some() {
                ctx.set_cursor_icon(egui::CursorIcon::Grabbing);
            } else if self.row_drag_source.is_some() {
                ctx.set_cursor_icon(egui::CursorIcon::Grabbing);
            } else if response.dragged() {
                ctx.set_cursor_icon(egui::CursorIcon::Grabbing);
            } else if self.protomatrix_state.hovered.is_some() {
                ctx.set_cursor_icon(egui::CursorIcon::PointingHand);
            } else if response.hovered() {
                ctx.set_cursor_icon(egui::CursorIcon::Grab);
            }

            // Inline text edit overlay for annotations
            if let Some(editing) = self.editing_annotation.take() {
                let (index, is_new, original) = match &editing {
                    EditingAnnotation::New { index } => (*index, true, String::new()),
                    EditingAnnotation::Existing { index, original } => (*index, false, original.clone()),
                };

                // Escape: cancel
                if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                    if is_new {
                        self.annotations.remove(index);
                    } else {
                        self.annotations[index].text = original;
                    }
                } else {
                    let pos_mm = (
                        self.annotations[index].pos_mm[0],
                        self.annotations[index].pos_mm[1],
                    );
                    let center_top = to_screen(pos_mm.0, pos_mm.1);
                    let font_id = egui::FontId::proportional(ANNOTATION_FONT_SIZE);
                    let line_height = ui.fonts_mut(|f| f.row_height(&font_id));
                    let min_w = 80.0;
                    let min_h = line_height.max(16.0);
                    // Measure text so edit box aligns with drawn text (CENTER_TOP).
                    // TextEdit is left-aligned, so put rect left at center - text_width/2.
                    let text = &self.annotations[index].text;
                    let text_width_px = if text.is_empty() {
                        min_w
                    } else {
                        ui.fonts_mut(|f| {
                            f.layout_no_wrap(
                                text.to_string(),
                                font_id.clone(),
                                egui::Color32::WHITE,
                            )
                            .size()
                            .x
                        })
                    };
                    let rect_w = text_width_px.max(min_w);
                    let rect_left = center_top.x - text_width_px / 2.0;
                    let edit_rect = egui::Rect::from_min_size(
                        egui::pos2(rect_left, center_top.y),
                        egui::vec2(rect_w, min_h),
                    );
                    let edit_id = egui::Id::new("annotation_edit").with(index);
                    let mut text = std::mem::take(&mut self.annotations[index].text);
                    let lost_focus = ui
                        .allocate_new_ui(egui::UiBuilder::new().max_rect(edit_rect), |ui| {
                            ui.visuals_mut().override_text_color = Some(egui::Color32::WHITE);
                            let te = egui::TextEdit::singleline(&mut text)
                                .frame(false)
                                .margin(egui::Margin::ZERO)
                                .font(font_id)
                                .desired_width(rect_w)
                                .id(edit_id);
                            let out = ui.add(te.background_color(egui::Color32::TRANSPARENT));
                            ui.memory_mut(|m| m.request_focus(edit_id));
                            out.lost_focus()
                        })
                        .inner;
                    self.annotations[index].text = text;

                    // Confirm on Enter or lost focus
                    let enter_pressed = ui.input(|i| i.key_pressed(egui::Key::Enter));
                    if enter_pressed || lost_focus {
                        if is_new && self.annotations[index].text.trim().is_empty() {
                            self.annotations.remove(index);
                        }
                    } else {
                        self.editing_annotation = Some(editing);
                    }
                }
            }

            // Process Disconnect from context menu (remove_temp consumes so we only process once)
            if let Some(col) = ctx.data_mut(|d| d.remove_temp::<Option<ColumnKey>>(disconnect_id)).flatten() {
                if let Some(ref js) = self.jumper_state {
                    migrate_disconnected_column_metadata(
                        col,
                        js,
                        self.protomatrix_config.matrix_size,
                        &mut self.net_names,
                        &mut self.net_colors,
                        &mut self.net_row_pins,
                    );
                }
                disconnect_column(&mut self.connections, col);
                self.connection_drag_source = None;
                self.selected_net = None;
                self.context_menu_column = None;
                self.run_autoroute();
            }
            // Process Remove from floating annotation context menu
            if let Some(idx) = ctx.data_mut(|d| d.remove_temp::<Option<usize>>(remove_annotation_id)).flatten() {
                if idx < self.annotations.len() {
                    self.annotations.remove(idx);
                    let editing_idx = self.editing_annotation.as_ref().map(|e| match e {
                        EditingAnnotation::New { index } => *index,
                        EditingAnnotation::Existing { index, .. } => *index,
                    });
                    if editing_idx == Some(idx) {
                        self.editing_annotation = None;
                    } else if let Some(editing_idx) = editing_idx {
                        if editing_idx > idx {
                            match self.editing_annotation.as_mut().unwrap() {
                                EditingAnnotation::New { index } => *index -= 1,
                                EditingAnnotation::Existing { index, .. } => *index -= 1,
                            }
                        }
                    }
                    self.context_menu_annotation = None;
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
                .and_then(|ni| {
                    self.jumper_state.as_ref().and_then(|js| {
                        js.net_canonical_keys.get(ni).map(|ck| {
                            let key = column_key_to_string(ck);
                            self.net_names
                                .get(&key)
                                .cloned()
                                .unwrap_or_else(|| column_key_to_x_label(ck))
                        })
                    })
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
                if self.row_drag_source.is_some() {
                    ui.label("(drag to row to swap nets)");
                }
                if let Some(ref err) = self.autoroute_error {
                    ui.colored_label(egui::Color32::RED, err);
                } else {
                    let nets_max = self.protomatrix_config.matrix_size as usize;
                    let (nets_used, jumpers_txt) = if let Some(ref js) = self.jumper_state {
                        (js.net_canonical_keys.len(), format!(" | Jumpers: {} closed", js.closed_count()))
                    } else {
                        (0, String::new())
                    };
                    ui.label(format!("| Nets: {} of {} used{}", nets_used, nets_max, jumpers_txt));
                }
                if ui.button("Clear connections").clicked() {
                    self.connections.clear();
                    self.connection_drag_source = None;
                    self.row_drag_source = None;
                    self.annotation_drag_source = None;
                    self.net_row_pins.clear();
                    self.jumper_state = None;
                    self.autoroute_error = None;
                    self.selected_net = None;
                }
            });
        });
    }
}
