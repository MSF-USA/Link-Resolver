//! Minimal desktop interface with bounded background work and copyable results.
//!
//! Every colour, size and font comes from [`crate::theme`]. Every control is reachable by
//! keyboard, the status line is a screen-reader live region, and nothing is shown only on hover.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::Duration;

use eframe::egui::{self, Color32, Id, Key, Modifiers, Response, RichText, Sense, Stroke};
use link_resolver::input::{Loaded, load_file, parse_urls};
use link_resolver::links::is_link;
use link_resolver::resolver::Resolver;
use link_resolver::results::{ExportFormat, Resolution, save_results};
use link_resolver::table::{PREVIEW_ROWS, Table, abbreviate};
use link_resolver::{APP_NAME, VERSION};

use crate::settings::Settings;
use crate::theme::{self, Palette, bold_text, secondary_text};

/// Events sent by workers; all UI state is updated on the main thread.
enum WorkerEvent {
    Loaded(Result<Option<Loaded>, String>),
    Saved(Result<Option<PathBuf>, String>),
    Resolved(Resolution),
    Finished { cancelled: bool },
    Failed(String),
}

/// Which card shows the status line. Messages appear next to the controls that caused them:
/// loading and resolving in the input card, finishing, copying and saving in the results card.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StatusArea {
    Input,
    Results,
}

/// Own the editable input, latest results and at most one background operation.
pub(crate) struct RedirectApp {
    input: String,
    results: Vec<Resolution>,
    status: String,
    is_error: bool,
    status_area: StatusArea,
    worker: Option<mpsc::Receiver<WorkerEvent>>,
    cancellation: Option<Arc<AtomicBool>>,
    total: usize,
    /// Set when a batch finishes so the next frame moves keyboard focus to the results summary.
    focus_results: bool,
    settings: Settings,
    /// Whether the Settings dialog is open over the page.
    settings_open: bool,
    /// A loaded table waiting for the user to choose its link column.
    column_choice: Option<ColumnChoice>,
    /// Where keyboard focus should go on the next frame after a dialog opens or closes.
    pending_focus: Option<FocusTarget>,
    /// Whether the Save dialog is open over the page.
    save_dialog_open: bool,
    /// The export chosen most recently in the Save dialog, offered first next time.
    export_format: ExportFormat,
}

/// A table whose link column could not be chosen automatically.
struct ColumnChoice {
    table: Table,
    /// Index of the column currently selected in the dialog.
    selected: usize,
}

/// Focus hand-offs that cross a frame boundary.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FocusTarget {
    /// The first control in the Settings dialog.
    SettingsDialog,
    /// The Settings button in the header.
    SettingsButton,
    /// The first control in the column chooser.
    ColumnDialog,
    /// The Load file button, where the column chooser came from.
    LoadButton,
    /// The first enabled choice in the Save dialog.
    SaveDialog,
    /// The Save button in the results header.
    SaveButton,
}

/// Width of the Settings dialog in points before clamping to the window.
const SETTINGS_DIALOG_WIDTH: f32 = 440.0;
/// Width of the Save dialog in points before clamping to the window.
const SAVE_DIALOG_WIDTH: f32 = 480.0;
/// Width of the column chooser; wide so a few columns of links fit without scrolling.
const COLUMN_DIALOG_WIDTH: f32 = 820.0;
/// Longest cell text shown in the column preview grid.
const PREVIEW_CELL_CHARS: usize = 40;
/// Space kept between a dialog and the window edges.
const DIALOG_MARGIN: f32 = 16.0;

/// Inner padding of the page's cards, and corner radius of their borders.
const CARD_PADDING: i8 = 16;
const CARD_RADIUS: u8 = 4;
/// Vertical space between the input card and the results card.
const SECTION_GAP: f32 = 14.0;
/// Below this width the results are stacked one block per link instead of a table, and the
/// results header puts its buttons on their own row.
const TABLE_MIN_WIDTH: f32 = 600.0;
/// Fixed results table columns; the original link and destination share what is left.
const NUMBER_COLUMN_WIDTH: f32 = 40.0;
const STATUS_COLUMN_WIDTH: f32 = 100.0;
const ACTION_COLUMN_WIDTH: f32 = 84.0;
/// Share of the flexible width given to the (truncated) original link.
const ORIGINAL_COLUMN_SHARE: f32 = 0.4;
/// Padding inside each table cell.
const CELL_PADDING_X: i8 = 12;
const CELL_PADDING_Y: i8 = 8;
/// Fixed id of the status line, which moves between cards but is one live region.
const STATUS_LINE_ID: &str = "status-line";

/// What a dialog's content reported, plus whether the dialog asked to close.
struct DialogResponse<T> {
    inner: T,
    /// Escape, a click on the backdrop, or `Ui::close`.
    dismissed: bool,
}

/// The table preview: a header row of radio buttons, one per column, over the first rows of
/// data. Clicking anywhere in a column selects it; the radio buttons carry keyboard focus.
/// The selected column is tinted with one rectangle painted behind the grid, so the tint
/// spans the column's full width however the cell contents vary.
fn draw_column_grid(
    ui: &mut egui::Ui,
    table: &Table,
    selected: &mut usize,
    palette: &Palette,
    controls: &mut Vec<Id>,
) {
    egui::ScrollArea::horizontal()
        .id_salt("column-grid")
        .show(ui, |ui| {
            // Cells size to their text; wrapping would squeeze headers into the width the
            // data cells had on the previous frame.
            ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
            let tint = ui.painter().add(egui::Shape::Noop);
            let mut tinted_area: Option<egui::Rect> = None;
            let mut clicked = None;
            egui::Grid::new("column-grid-cells")
                .spacing([0.0, 0.0])
                .show(ui, |ui| {
                    for (index, column) in table.columns.iter().enumerate() {
                        let cell = grid_cell(ui, |ui| {
                            ui.vertical(|ui| {
                                let radio = ui.radio_value(
                                    selected,
                                    index,
                                    bold_text(&column.name, theme::BODY_SIZE, palette),
                                );
                                theme::focus_ring(ui, &radio, palette.focus_ring);
                                controls.push(radio.id);
                                ui.label(secondary_text(&column.summary(), palette));
                            });
                        });
                        note_cell(&cell, index, *selected, &mut tinted_area, &mut clicked);
                    }
                    ui.end_row();
                    for row in table.rows().iter().take(PREVIEW_ROWS) {
                        for (index, value) in row.iter().enumerate() {
                            let will_skip =
                                index == *selected && !value.is_empty() && !is_link(value);
                            let cell = grid_cell(ui, |ui| {
                                let mut text = RichText::new(abbreviate(value, PREVIEW_CELL_CHARS))
                                    .color(palette.text);
                                if will_skip {
                                    text = text.strikethrough().color(palette.text_secondary);
                                }
                                ui.label(text);
                            });
                            note_cell(&cell, index, *selected, &mut tinted_area, &mut clicked);
                        }
                        ui.end_row();
                    }
                });
            if let Some(area) = tinted_area {
                ui.painter()
                    .set(tint, egui::Shape::rect_filled(area, 3.0, palette.highlight));
            }
            if let Some(index) = clicked {
                *selected = index;
            }
        });
}

/// Record a drawn cell: grow the tinted area if it is in the selected column, and remember
/// a click on it as a selection.
fn note_cell(
    cell: &Response,
    column: usize,
    selected: usize,
    tinted_area: &mut Option<egui::Rect>,
    clicked: &mut Option<usize>,
) {
    if column == selected {
        *tinted_area = Some(tinted_area.map_or(cell.rect, |area| area.union(cell.rect)));
    }
    if cell.clicked() {
        *clicked = Some(column);
    }
}

/// One padded, clickable grid cell.
fn grid_cell(ui: &mut egui::Ui, content: impl FnOnce(&mut egui::Ui)) -> Response {
    egui::Frame::new()
        .inner_margin(egui::Margin::symmetric(10, 5))
        .show(ui, content)
        .response
        .interact(Sense::click())
}

/// Show a centred dialog over the page and keep keyboard focus inside it.
///
/// `content` draws the dialog body and pushes the id of every focusable control it creates,
/// in Tab order, into the vector it is given. Focus moves to the first control when
/// `focus_into` is true, and comes back inside whenever Tab wraps out or focus is lost.
fn show_dialog<T>(
    context: &egui::Context,
    palette: &Palette,
    salt: &str,
    width: f32,
    focus_into: bool,
    content: impl FnOnce(&mut egui::Ui, &mut Vec<Id>) -> T,
) -> DialogResponse<T> {
    let window = context.content_rect();
    let width = width.min(window.width() - 2.0 * DIALOG_MARGIN);
    let max_height = window.height() - 2.0 * DIALOG_MARGIN;
    let mut controls: Vec<Id> = Vec::new();
    let modal = egui::Modal::new(Id::unique(salt))
        .backdrop_color(Color32::from_black_alpha(110))
        .frame(
            egui::Frame::new()
                .fill(palette.surface)
                .stroke(Stroke::new(1.0, palette.control_border))
                .corner_radius(6)
                .inner_margin(20),
        )
        .show(context, |ui| {
            ui.set_width(width);
            egui::ScrollArea::vertical()
                .id_salt(salt)
                .max_height(max_height)
                .show(ui, |ui| content(ui, &mut controls))
                .inner
        });
    let dismissed = modal.should_close();
    if !dismissed {
        let focused = context.memory(|memory| memory.focused());
        let outside = focused.is_none_or(|id| !controls.contains(&id));
        if (focus_into || outside)
            && let Some(first) = controls.first()
        {
            context.memory_mut(|memory| memory.request_focus(*first));
        }
    }
    DialogResponse {
        inner: modal.inner,
        dismissed,
    }
}

impl RedirectApp {
    /// Create an idle app, install the brand fonts and both theme palettes, and apply the
    /// saved settings (defaults when `storage` is `None`). This starts no network requests.
    pub(crate) fn new(context: &egui::Context, storage: Option<&dyn eframe::Storage>) -> Self {
        theme::install(context);
        let settings = Settings::load(storage);
        settings.apply(context);
        Self {
            input: String::new(),
            results: Vec::new(),
            status: "Paste links or load a text file to begin.".into(),
            is_error: false,
            status_area: StatusArea::Input,
            worker: None,
            cancellation: None,
            total: 0,
            focus_results: false,
            settings,
            settings_open: false,
            column_choice: None,
            pending_focus: None,
            save_dialog_open: false,
            export_format: ExportFormat::default(),
        }
    }

    /// Whether a dialog is covering the page.
    fn dialog_open(&self) -> bool {
        self.settings_open || self.column_choice.is_some() || self.save_dialog_open
    }

    /// Move keyboard focus to `response` if `target` is the pending hand-off.
    fn deliver_focus(&mut self, target: FocusTarget, response: &Response) {
        if self.pending_focus == Some(target) {
            response.request_focus();
            self.pending_focus = None;
        }
    }

    fn open_settings(&mut self) {
        self.settings_open = true;
        self.pending_focus = Some(FocusTarget::SettingsDialog);
    }

    fn close_settings(&mut self) {
        self.settings_open = false;
        self.pending_focus = Some(FocusTarget::SettingsButton);
    }

    /// Report on loading or resolving, in the input card.
    fn set_status(&mut self, message: impl Into<String>, is_error: bool) {
        self.status = message.into();
        self.is_error = is_error;
        self.status_area = StatusArea::Input;
    }

    /// Report on finished batches, copying or saving, in the results card.
    fn set_results_status(&mut self, message: impl Into<String>, is_error: bool) {
        self.set_status(message, is_error);
        self.status_area = StatusArea::Results;
    }

    fn open_save_dialog(&mut self) {
        if !self.has_resolved_links() {
            self.export_format = ExportFormat::Table;
        }
        self.save_dialog_open = true;
        self.pending_focus = Some(FocusTarget::SaveDialog);
    }

    fn close_save_dialog(&mut self) {
        self.save_dialog_open = false;
        self.pending_focus = Some(FocusTarget::SaveButton);
    }

    fn has_resolved_links(&self) -> bool {
        self.results.iter().any(|result| result.outcome.is_ok())
    }

    fn launch_worker(
        &mut self,
        context: &egui::Context,
        work: impl FnOnce(mpsc::Sender<WorkerEvent>, egui::Context) + Send + 'static,
    ) -> bool {
        let (sender, receiver) = mpsc::channel();
        let context = context.clone();
        match thread::Builder::new()
            .name("link-resolver-worker".into())
            .spawn(move || work(sender, context))
        {
            Ok(_) => {
                self.worker = Some(receiver);
                true
            }
            Err(error) => {
                self.set_status(format!("Could not start background work: {error}"), true);
                false
            }
        }
    }

    fn start_resolution(&mut self, context: &egui::Context) {
        if self.worker.is_some() {
            return;
        }
        let urls = parse_urls(&self.input);
        if urls.is_empty() {
            self.set_status(
                "Enter at least one full link starting with https:// or http://.",
                true,
            );
            return;
        }
        let total = urls.len();
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = cancel.clone();
        if self.launch_worker(context, move |sender, context| {
            resolve_in_background(urls, worker_cancel, sender, context);
        }) {
            self.results.clear();
            self.total = total;
            self.cancellation = Some(cancel);
            self.set_status(format!("Resolving 0 of {total} links…"), false);
        }
    }

    fn load_file(&mut self, context: &egui::Context) {
        if self.worker.is_some() {
            return;
        }
        if self.launch_worker(context, |sender, context| {
            let outcome = rfd::FileDialog::new()
                .set_title("Load links from a text or spreadsheet file")
                .add_filter("Text and spreadsheet files", &["txt", "csv", "tsv"])
                .add_filter("Text files", &["txt"])
                .add_filter("CSV and TSV files", &["csv", "tsv"])
                .add_filter("All files", &["*"])
                .pick_file()
                .map(|path| load_file(&path))
                .transpose();
            notify(&sender, &context, WorkerEvent::Loaded(outcome));
        }) {
            self.set_status(
                "Choose a text file with one link per line, or a CSV or TSV file.",
                false,
            );
        }
    }

    /// Ask for a new file name on a worker thread and save the latest batch in `format`.
    fn save_results(&mut self, context: &egui::Context, format: ExportFormat) {
        if self.worker.is_some() || self.results.is_empty() {
            return;
        }
        let results = self.results.clone();
        if self.launch_worker(context, move |sender, context| {
            let outcome = rfd::FileDialog::new()
                .set_title("Save results — choose a new file name")
                .add_filter(format.file_type(), &[format.extension()])
                .set_file_name(format.default_file_name())
                .save_file()
                .map(|mut path| {
                    if path.extension().is_none() {
                        path.set_extension(format.extension());
                    }
                    save_results(&path, &results, format).map(|()| path)
                })
                .transpose();
            notify(&sender, &context, WorkerEvent::Saved(outcome));
        }) {
            self.set_results_status(
                "Choose where to save the results. Existing files are kept.",
                false,
            );
        }
    }

    fn poll_worker(&mut self) {
        while let Some(receiver) = &self.worker {
            match receiver.try_recv() {
                Ok(event) => self.apply_event(event),
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.worker = None;
                    self.cancellation = None;
                    self.set_status("Resolving stopped early. Click Resolve to try again.", true);
                    break;
                }
            }
        }
    }

    fn apply_event(&mut self, event: WorkerEvent) {
        match event {
            WorkerEvent::Resolved(result) => {
                self.results.push(result);
                self.set_status(
                    format!("Resolving {} of {} links…", self.results.len(), self.total),
                    false,
                );
                return;
            }
            WorkerEvent::Loaded(Ok(Some(Loaded::Table(table)))) => self.receive_table(table),
            WorkerEvent::Loaded(Ok(Some(Loaded::Links(urls)))) => {
                self.input = urls.join("\n");
                self.set_status(
                    format!("Loaded {} links. Click Resolve to start.", urls.len()),
                    false,
                );
            }
            WorkerEvent::Loaded(Ok(None)) => self.set_status("Load cancelled.", false),
            WorkerEvent::Loaded(Err(error)) => self.set_status(error, true),
            WorkerEvent::Saved(Ok(Some(path))) => {
                self.set_results_status(format!("Saved results to {}.", path.display()), false);
            }
            WorkerEvent::Saved(Ok(None)) => self.set_results_status("Save cancelled.", false),
            WorkerEvent::Saved(Err(error)) => self.set_results_status(error, true),
            WorkerEvent::Failed(error) => self.set_status(error, true),
            WorkerEvent::Finished { cancelled } => {
                let succeeded = self
                    .results
                    .iter()
                    .filter(|result| result.outcome.is_ok())
                    .count();
                let failed = self.results.len() - succeeded;
                let prefix = if cancelled { "Stopped" } else { "Finished" };
                self.set_results_status(
                    format!("{prefix}: {succeeded} resolved, {failed} failed."),
                    failed > 0,
                );
                self.focus_results = true;
            }
        }
        self.worker = None;
        self.cancellation = None;
    }

    fn draw(&mut self, ui: &mut egui::Ui) {
        self.poll_worker();
        if self.worker.is_some() {
            // Also detect worker panics/disconnections without waiting for input.
            ui.ctx().request_repaint_after(Duration::from_millis(100));
        }
        // Ctrl+Enter (Cmd+Enter on macOS) starts a batch from inside the text box.
        if self.worker.is_none()
            && !self.dialog_open()
            && ui.input_mut(|input| input.consume_key(Modifiers::COMMAND, Key::Enter))
        {
            self.start_resolution(ui.ctx());
        }
        if self.settings.handle_shortcuts(ui.ctx()) {
            self.settings.apply(ui.ctx());
        }
        let palette = Palette::for_theme(ui.ctx().theme());
        let idle = self.worker.is_none();

        // One page-level scroll area keeps every control reachable at high zoom or in a
        // small window; nested scroll regions would trap keyboard users.
        egui::ScrollArea::vertical()
            .id_salt("page")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                draw_brand_rule(ui, &palette);
                ui.horizontal(|ui| {
                    ui.heading(APP_NAME);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let button = secondary_button(ui, true, "Settings", &palette)
                            .on_hover_text("Theme, text size and motion.");
                        self.deliver_focus(FocusTarget::SettingsButton, &button);
                        if button.clicked() {
                            self.open_settings();
                        }
                    });
                });
                ui.label(secondary_text(
                    "Find the final destination behind any redirect link, including Vertex AI grounding links.",
                    &palette,
                ));
                ui.add_space(10.0);
                self.draw_input_card(ui, &palette, idle);
                ui.add_space(SECTION_GAP);
                self.draw_results_card(ui, &palette, idle);
                ui.add_space(5.0);
                ui.label(secondary_text(
                    "Only use links you trust. Each link can take up to 30 seconds.",
                    &palette,
                ));
            });
        if self.settings_open {
            self.draw_settings_dialog(ui.ctx(), &palette);
        } else if self.column_choice.is_some() {
            self.draw_column_dialog(ui.ctx(), &palette);
        } else if self.save_dialog_open {
            self.draw_save_dialog(ui.ctx(), &palette);
        }
    }

    /// Choose what to save: the whole table as CSV, or only the resolved links as text.
    /// "Save…" closes the dialog and opens the system file chooser.
    fn draw_save_dialog(&mut self, context: &egui::Context, palette: &Palette) {
        let focus_into = self.pending_focus == Some(FocusTarget::SaveDialog);
        let has_resolved = self.has_resolved_links();
        let format = &mut self.export_format;
        let dialog = show_dialog(
            context,
            palette,
            "save-dialog",
            SAVE_DIALOG_WIDTH,
            focus_into,
            |ui, controls| {
                ui.label(bold_text("Save results", 20.0, palette));
                ui.add_space(10.0);
                for option in ExportFormat::ALL {
                    let enabled = option == ExportFormat::Table || has_resolved;
                    let (title, description) = export_choice_text(option, has_resolved);
                    ui.add_enabled_ui(enabled, |ui| {
                        let radio = ui.radio_value(format, option, title);
                        theme::focus_ring(ui, &radio, palette.focus_ring);
                        if enabled {
                            controls.push(radio.id);
                        }
                        ui.indent(title, |ui| {
                            ui.label(secondary_text(description, palette));
                        });
                    });
                    ui.add_space(6.0);
                }
                ui.add_space(8.0);
                ui.separator();
                let mut outcome = None;
                ui.horizontal(|ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let save = primary_button(ui, true, "Save…", palette)
                            .on_hover_text("Choose a new file name for the selected format.");
                        let cancel = secondary_button(ui, true, "Cancel", palette)
                            .on_hover_text("Close without saving (Escape).");
                        // Tab order follows reading order, so Cancel comes before Save.
                        controls.push(cancel.id);
                        controls.push(save.id);
                        if save.clicked() {
                            outcome = Some(true);
                        } else if cancel.clicked() {
                            outcome = Some(false);
                        }
                    });
                });
                outcome
            },
        );
        if focus_into {
            self.pending_focus = None;
        }
        match dialog.inner {
            Some(true) => {
                self.close_save_dialog();
                self.save_results(context, self.export_format);
            }
            Some(false) => self.close_save_dialog(),
            None if dialog.dismissed => self.close_save_dialog(),
            None => {}
        }
    }

    /// Settings over the page: closes on Done, Escape or a click outside.
    fn draw_settings_dialog(&mut self, context: &egui::Context, palette: &Palette) {
        let focus_into = self.pending_focus == Some(FocusTarget::SettingsDialog);
        let settings = &mut self.settings;
        let dialog = show_dialog(
            context,
            palette,
            "settings-dialog",
            SETTINGS_DIALOG_WIDTH,
            focus_into,
            |ui, controls| {
                ui.label(bold_text("Settings", 20.0, palette));
                ui.add_space(10.0);
                if settings.draw(ui, palette, controls) {
                    settings.apply(ui.ctx());
                }
                ui.add_space(14.0);
                ui.separator();
                let mut done = false;
                ui.horizontal(|ui| {
                    ui.label(secondary_text(
                        &format!("Your choices are remembered. Version {VERSION}."),
                        palette,
                    ));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let button = primary_button(ui, true, "Done", palette)
                            .on_hover_text("Close settings (Escape).");
                        controls.push(button.id);
                        done = button.clicked();
                    });
                });
                done
            },
        );
        if focus_into {
            self.pending_focus = None;
        }
        if dialog.inner || dialog.dismissed {
            self.close_settings();
        }
    }

    /// Ask which column of a loaded table holds the links. Shown when the table has no
    /// column made only of links, or more than one. The table is previewed as a grid; the
    /// selected column is tinted and its cells that would be skipped are struck through.
    fn draw_column_dialog(&mut self, context: &egui::Context, palette: &Palette) {
        let focus_into = self.pending_focus == Some(FocusTarget::ColumnDialog);
        let Some(choice) = self.column_choice.as_mut() else {
            return;
        };
        let dialog = show_dialog(
            context,
            palette,
            "column-dialog",
            COLUMN_DIALOG_WIDTH,
            focus_into,
            |ui, controls| {
                ui.label(bold_text("Choose the column with links", 20.0, palette));
                ui.add_space(4.0);
                let link_columns = choice.table.link_columns();
                let rows = choice.table.row_count();
                let guidance = if link_columns.is_empty() {
                    "No column holds only links. Choose the closest one; rows without a link will be skipped."
                } else {
                    "Several columns hold links. Choose the one to resolve."
                };
                ui.label(secondary_text(
                    &format!(
                        "{} has {rows} row{}. {guidance}",
                        choice.table.name,
                        if rows == 1 { "" } else { "s" }
                    ),
                    palette,
                ));
                ui.add_space(8.0);
                let mut has_header = choice.table.has_header();
                let header_toggle = ui.checkbox(&mut has_header, "First row is a header");
                theme::focus_ring(ui, &header_toggle, palette.focus_ring);
                controls.push(header_toggle.id);
                if header_toggle.changed() {
                    choice.table.set_header(has_header);
                }
                ui.add_space(8.0);
                let ColumnChoice { table, selected } = choice;
                draw_column_grid(ui, table, selected, palette, controls);
                if rows > PREVIEW_ROWS {
                    ui.label(secondary_text(
                        &format!("… and {} more rows", rows - PREVIEW_ROWS),
                        palette,
                    ));
                }
                ui.add_space(8.0);
                ui.separator();
                let mut outcome = None;
                // A horizontal row keeps the footer one line tall; a bare right-to-left layout
                // would claim the rest of the scroll area and push the buttons out of view.
                ui.horizontal(|ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let use_column = primary_button(ui, true, "Use column", palette)
                            .on_hover_text("Load the links from the selected column.");
                        let cancel = secondary_button(ui, true, "Cancel", palette)
                            .on_hover_text("Leave the text box unchanged (Escape).");
                        // Tab order follows reading order, so Cancel comes before Use column.
                        controls.push(cancel.id);
                        controls.push(use_column.id);
                        if use_column.clicked() {
                            outcome = Some(true);
                        } else if cancel.clicked() {
                            outcome = Some(false);
                        }
                    });
                });
                outcome
            },
        );
        if focus_into {
            self.pending_focus = None;
        }
        match dialog.inner {
            Some(true) => self.use_column(),
            Some(false) => self.cancel_column_choice(),
            None if dialog.dismissed => self.cancel_column_choice(),
            None => {}
        }
    }

    /// Put the selected column's links in the text box and report skipped rows.
    fn use_column(&mut self) {
        if let Some(choice) = self.column_choice.take() {
            self.take_column(&choice.table, choice.selected);
        }
        self.pending_focus = Some(FocusTarget::LoadButton);
    }

    fn cancel_column_choice(&mut self) {
        self.column_choice = None;
        self.set_status("Load cancelled. The text box is unchanged.", false);
        self.pending_focus = Some(FocusTarget::LoadButton);
    }

    /// Replace the text box with one column's links. Rows that hold something other than a
    /// link are reported as a problem so nobody misses them.
    fn take_column(&mut self, table: &Table, column: usize) {
        let extraction = table.extract(column);
        self.input = extraction.links.join("\n");
        let mut status = extraction.describe(&table.columns[column].name);
        if extraction.links.is_empty() {
            status.push_str(" Nothing to resolve.");
        } else {
            status.push_str(" Click Resolve to start.");
        }
        self.set_status(status, !extraction.skipped_rows.is_empty());
    }

    /// Decide what to do with a loaded table: use its only link column, or ask.
    fn receive_table(&mut self, table: Table) {
        let link_columns = table.link_columns();
        if let [only] = link_columns[..] {
            self.take_column(&table, only);
            return;
        }
        self.column_choice = Some(ColumnChoice {
            selected: link_columns.first().copied().unwrap_or(0),
            table,
        });
        self.pending_focus = Some(FocusTarget::ColumnDialog);
    }

    /// The input card: the link box with its own actions, status line and progress bar.
    fn draw_input_card(&mut self, ui: &mut egui::Ui, palette: &Palette, idle: bool) {
        card_frame(palette)
            .inner_margin(CARD_PADDING)
            .show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                let heading = ui.label(bold_text(
                    "Links to resolve",
                    theme::SECTION_HEADING_SIZE,
                    palette,
                ));
                ui.label(secondary_text(
                    "One full link per line, or load a text or spreadsheet file.",
                    palette,
                ));
                ui.add_enabled_ui(idle, |ui| {
                    egui::ScrollArea::vertical()
                        .id_salt("url-input-scroll")
                        .max_height(155.0)
                        .show(ui, |ui| {
                            ui.add(
                                egui::TextEdit::multiline(&mut self.input)
                                    .hint_text("https://vertexaisearch.cloud.google.com/grounding-api-redirect/…\nhttps://example.com/redirect")
                                    .desired_rows(4)
                                    .desired_width(f32::INFINITY),
                            )
                            .labelled_by(heading.id);
                        });
                });
                ui.add_space(4.0);
                self.draw_input_actions(ui, palette, idle);
                if self.status_area == StatusArea::Input {
                    ui.add_space(4.0);
                    self.draw_status_line(ui, palette);
                }
                if self.cancellation.is_some() {
                    let fraction = self.results.len() as f32 / self.total.max(1) as f32;
                    ui.add(
                        egui::ProgressBar::new(fraction)
                            .desired_height(6.0)
                            .fill(palette.accent),
                    );
                }
            });
    }

    /// Load file on the left; Resolve, and Stop while a batch runs, on the right.
    fn draw_input_actions(&mut self, ui: &mut egui::Ui, palette: &Palette, idle: bool) {
        ui.horizontal(|ui| {
            let load = secondary_button(ui, idle, "Load file…", palette).on_hover_text(
                "Choose a text file with one link per line, or a CSV or TSV spreadsheet file.",
            );
            self.deliver_focus(FocusTarget::LoadButton, &load);
            if load.clicked() {
                self.load_file(ui.ctx());
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if primary_button(ui, idle, "Resolve", palette)
                    .on_hover_text("Find the destination of every link (Ctrl+Enter).")
                    .clicked()
                {
                    self.start_resolution(ui.ctx());
                }
                if let Some(cancel) = &self.cancellation {
                    let stopping = cancel.load(Ordering::Relaxed);
                    if secondary_button(ui, !stopping, "Stop", palette)
                        .on_hover_text("Stop after the current link finishes (up to 30 seconds).")
                        .clicked()
                    {
                        cancel.store(true, Ordering::Relaxed);
                        self.set_status("Stopping after the current link finishes…", false);
                    }
                }
            });
        });
    }

    /// The one status line, drawn in whichever card [`StatusArea`] names. Its label keeps a
    /// fixed id so assistive technology sees one live region that changes, wherever it is.
    fn draw_status_line(&self, ui: &mut egui::Ui, palette: &Palette) {
        ui.horizontal_wrapped(|ui| {
            if self.worker.is_some() && !self.settings.reduce_motion {
                ui.spinner();
            }
            let color = if self.is_error {
                palette.problem
            } else {
                palette.text
            };
            ui.scope_builder(
                egui::UiBuilder::new().scope_id(Id::unique(STATUS_LINE_ID)),
                |ui| {
                    let status =
                        ui.add(egui::Label::new(RichText::new(&self.status).color(color)).wrap());
                    // Screen readers announce changes to this line without moving focus.
                    ui.ctx().accesskit_node_builder(status.id, |node| {
                        node.set_live(egui::accesskit::Live::Polite);
                    });
                },
            );
        });
    }

    /// The results card: a header bar with the summary and the batch actions, then the table.
    fn draw_results_card(&mut self, ui: &mut egui::Ui, palette: &Palette, idle: bool) {
        card_frame(palette).show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 0.0;
            self.draw_results_header(ui, palette, idle);
            let padded = egui::Frame::new().inner_margin(egui::Margin::symmetric(
                CARD_PADDING,
                CELL_PADDING_Y,
            ));
            if self.status_area == StatusArea::Results {
                padded.show(ui, |ui| {
                    ui.set_min_width(ui.available_width());
                    self.draw_status_line(ui, palette);
                });
                draw_row_rule(ui, palette);
            }
            if self.results.is_empty() {
                padded.show(ui, |ui| {
                    ui.set_min_width(ui.available_width());
                    ui.label(secondary_text(
                        "Resolved links will appear here. You can copy them or save them to a file.",
                        palette,
                    ));
                });
                return;
            }
            let copied = if ui.available_width() >= TABLE_MIN_WIDTH {
                draw_results_table(ui, &self.results, palette)
            } else {
                draw_results_stacked(ui, &self.results, palette)
            };
            if let Some(index) = copied {
                self.set_results_status(
                    format!("Copied the destination of link {}.", index + 1),
                    false,
                );
            }
        });
    }

    /// Title, summary, Copy resolved links and Save on a grey bar. The title takes keyboard
    /// focus when a batch finishes and its accessible name includes the summary.
    fn draw_results_header(&mut self, ui: &mut egui::Ui, palette: &Palette, idle: bool) {
        let summary = self.results_summary();
        egui::Frame::new()
            .fill(palette.section_header)
            .corner_radius(egui::CornerRadius {
                nw: CARD_RADIUS,
                ne: CARD_RADIUS,
                sw: 0,
                se: 0,
            })
            .inner_margin(egui::Margin::symmetric(CARD_PADDING, 10))
            .show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                ui.spacing_mut().item_spacing.y = 8.0;
                let wide = ui.available_width() >= TABLE_MIN_WIDTH;
                let title_row = |ui: &mut egui::Ui| {
                    let title = ui.add(
                        egui::Label::new(bold_text(
                            "Results",
                            theme::SECTION_HEADING_SIZE,
                            palette,
                        ))
                        .sense(Sense::focusable_noninteractive()),
                    );
                    ui.ctx().accesskit_node_builder(title.id, |node| {
                        node.set_label(format!("Results: {summary}"));
                    });
                    ui.add_space(8.0);
                    ui.label(secondary_text(&summary, palette));
                    title
                };
                // Labels are placed before the buttons, so the row is given the button height up
                // front; otherwise the title sits at the top while the buttons are centred.
                let button_height = ui.text_style_height(&egui::TextStyle::Button)
                    + 2.0 * ui.spacing().button_padding.y;
                let title = if wide {
                    ui.horizontal(|ui| {
                        ui.set_min_height(button_height);
                        let title = title_row(ui);
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            self.draw_results_actions(ui, palette, idle);
                        });
                        title
                    })
                    .inner
                } else {
                    let title = ui.horizontal_wrapped(title_row).inner;
                    ui.horizontal_wrapped(|ui| {
                        self.draw_results_actions(ui, palette, idle);
                    });
                    title
                };
                if self.focus_results {
                    title.request_focus();
                    self.focus_results = false;
                }
                theme::focus_ring(ui, &title, palette.focus_ring);
            });
    }

    /// Copy resolved links and Save. In a right-to-left layout Save is added first so it
    /// sits at the far right; in a wrapped row they read Copy then Save.
    fn draw_results_actions(&mut self, ui: &mut egui::Ui, palette: &Palette, idle: bool) {
        let right_to_left = ui.layout().main_dir() == egui::Direction::RightToLeft;
        let can_save = idle && !self.results.is_empty();
        let can_copy = self.has_resolved_links();
        let save = |app: &mut Self, ui: &mut egui::Ui| {
            let button = secondary_button(ui, can_save, "Save…", palette).on_hover_text(
                "Save the whole table as a CSV spreadsheet, or only the resolved links as a text file.",
            );
            app.deliver_focus(FocusTarget::SaveButton, &button);
            if button.clicked() {
                app.open_save_dialog();
            }
        };
        let copy = |app: &mut Self, ui: &mut egui::Ui| {
            if secondary_button(ui, can_copy, "Copy resolved links", palette)
                .on_hover_text("Copy every resolved link to the clipboard, one per line.")
                .clicked()
            {
                ui.ctx().copy_text(resolved_urls(&app.results));
                app.set_results_status("Resolved links copied. Paste them anywhere.", false);
            }
        };
        if right_to_left {
            save(self, ui);
            copy(self, ui);
        } else {
            copy(self, ui);
            save(self, ui);
        }
    }

    /// "3 links · 2 resolved · 1 failed", or progress while a batch runs.
    fn results_summary(&self) -> String {
        if self.cancellation.is_some() {
            return format!("{} of {} done", self.results.len(), self.total);
        }
        if self.results.is_empty() {
            return "No results yet".into();
        }
        let resolved = self
            .results
            .iter()
            .filter(|result| result.outcome.is_ok())
            .count();
        let total = self.results.len();
        format!(
            "{total} link{} · {resolved} resolved · {} failed",
            if total == 1 { "" } else { "s" },
            total - resolved
        )
    }
}

impl eframe::App for RedirectApp {
    fn logic(&mut self, context: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_worker();
        if self.worker.is_some() {
            context.request_repaint_after(Duration::from_millis(100));
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::Frame::central_panel(ui.style())
            .inner_margin(egui::Margin::same(20))
            .show(ui, |ui| self.draw(ui));
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        self.settings.save(storage);
    }

    /// Settings are the source of truth for zoom and theme, so egui's own memory is not saved.
    fn persist_egui_memory(&self) -> bool {
        false
    }
}

impl Drop for RedirectApp {
    fn drop(&mut self) {
        if let Some(cancel) = &self.cancellation {
            cancel.store(true, Ordering::Relaxed);
        }
    }
}

fn notify(sender: &mpsc::Sender<WorkerEvent>, context: &egui::Context, event: WorkerEvent) -> bool {
    let delivered = sender.send(event).is_ok();
    context.request_repaint();
    delivered
}

fn resolve_in_background(
    urls: Vec<String>,
    cancel: Arc<AtomicBool>,
    sender: mpsc::Sender<WorkerEvent>,
    context: egui::Context,
) {
    let resolver = match Resolver::new() {
        Ok(resolver) => resolver,
        Err(error) => {
            notify(&sender, &context, WorkerEvent::Failed(error));
            return;
        }
    };
    for url in urls {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        if !notify(
            &sender,
            &context,
            WorkerEvent::Resolved(resolver.resolve(&url)),
        ) {
            return;
        }
    }
    notify(
        &sender,
        &context,
        WorkerEvent::Finished {
            cancelled: cancel.load(Ordering::Relaxed),
        },
    );
}

fn resolved_urls(results: &[Resolution]) -> String {
    results
        .iter()
        .filter_map(|result| result.outcome.as_ref().ok())
        .map(String::as_str)
        .collect::<Vec<_>>()
        .join("\n")
}

/// The short red rule the brand places above headings.
fn draw_brand_rule(ui: &mut egui::Ui, palette: &Palette) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(48.0, 6.0), Sense::hover());
    ui.painter().rect_filled(rect, 0.0, palette.accent);
    ui.add_space(4.0);
}

/// A white-surface button with a border, plus a focus ring when it has keyboard focus.
fn secondary_button(ui: &mut egui::Ui, enabled: bool, label: &str, palette: &Palette) -> Response {
    let response = ui.add_enabled(enabled, egui::Button::new(label));
    theme::focus_ring(ui, &response, palette.focus_ring);
    response
}

/// The red primary action. The pressed colour comes from last frame's response, since the
/// fill must be chosen before the button is laid out.
fn primary_button(ui: &mut egui::Ui, enabled: bool, label: &str, palette: &Palette) -> Response {
    let id = ui.next_auto_id();
    let pressed = ui
        .ctx()
        .read_response(id)
        .is_some_and(|previous| previous.is_pointer_button_down_on());
    let fill = if pressed {
        palette.accent_pressed
    } else {
        palette.accent
    };
    let response = ui.add_enabled(
        enabled,
        egui::Button::new(
            RichText::new(label)
                .font(theme::bold(theme::BODY_SIZE))
                .color(palette.on_accent),
        )
        .fill(fill)
        .stroke(Stroke::NONE),
    );
    theme::focus_ring(ui, &response, palette.focus_ring_on_accent);
    response
}

/// Title and description of one choice in the Save dialog.
fn export_choice_text(format: ExportFormat, has_resolved: bool) -> (&'static str, &'static str) {
    match format {
        ExportFormat::Table => (
            "Whole table (CSV)",
            "Every link with its destination, status and any error. Opens in Excel or LibreOffice.",
        ),
        ExportFormat::ResolvedLinks if has_resolved => (
            "Resolved links only (text file)",
            "One destination per line, ready to paste or load back in. Failed links are left out.",
        ),
        ExportFormat::ResolvedLinks => (
            "Resolved links only (text file)",
            "Not available: no link in this batch was resolved.",
        ),
    }
}

/// A bordered white (or dark surface) card holding one section of the page.
fn card_frame(palette: &Palette) -> egui::Frame {
    egui::Frame::new()
        .fill(palette.surface)
        .stroke(Stroke::new(1.0, palette.border))
        .corner_radius(CARD_RADIUS)
}

/// A full-width hairline between table rows, in the decorative border colour.
fn draw_row_rule(ui: &mut egui::Ui, palette: &Palette) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter().hline(
        rect.x_range(),
        rect.center().y,
        Stroke::new(1.0, palette.border),
    );
}

/// Widths of the results table columns for a given table width. Number, status and the Copy
/// button are fixed; the original link and destination share the rest, the destination
/// getting the larger part because it is never truncated.
#[derive(Clone, Copy, Debug, PartialEq)]
struct ColumnWidths([f32; 5]);

impl ColumnWidths {
    fn for_width(width: f32) -> Self {
        let fixed = NUMBER_COLUMN_WIDTH + STATUS_COLUMN_WIDTH + ACTION_COLUMN_WIDTH;
        let flexible = (width - fixed).max(0.0);
        let original = (flexible * ORIGINAL_COLUMN_SHARE).floor();
        Self([
            NUMBER_COLUMN_WIDTH,
            STATUS_COLUMN_WIDTH,
            original,
            flexible - original,
            ACTION_COLUMN_WIDTH,
        ])
    }
}

/// One table row: each cell is a fixed-width, top-aligned, padded column. `cell` draws the
/// content of column `index`.
fn draw_table_row(
    ui: &mut egui::Ui,
    widths: ColumnWidths,
    mut cell: impl FnMut(&mut egui::Ui, usize),
) {
    let spacing = ui.spacing().item_spacing;
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        for (index, width) in widths.0.into_iter().enumerate() {
            ui.allocate_ui_with_layout(
                egui::vec2(width, 0.0),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    ui.set_width(width);
                    egui::Frame::new()
                        .inner_margin(egui::Margin::symmetric(CELL_PADDING_X, CELL_PADDING_Y))
                        .show(ui, |ui| {
                            ui.spacing_mut().item_spacing = spacing;
                            ui.set_width(ui.available_width());
                            cell(ui, index);
                        });
                },
            );
        }
    });
}

/// The results as a table. Returns the index of a row whose Copy button was clicked.
fn draw_results_table(
    ui: &mut egui::Ui,
    results: &[Resolution],
    palette: &Palette,
) -> Option<usize> {
    let widths = ColumnWidths::for_width(ui.available_width());
    let headings = ["#", "Status", "Original link", "Destination", ""];
    draw_table_row(ui, widths, |ui, column| {
        ui.label(bold_text(headings[column], theme::BODY_SIZE, palette));
    });
    draw_row_rule(ui, palette);
    let mut copied = None;
    for (index, result) in results.iter().enumerate() {
        draw_table_row(ui, widths, |ui, column| match column {
            0 => {
                ui.label(format!("{}", index + 1));
            }
            1 => {
                ui.label(status_text(result, palette));
            }
            2 => original_link(ui, result, palette),
            3 => destination(ui, result),
            _ => {
                if copy_button(ui, index, result, palette) {
                    copied = Some(index);
                }
            }
        });
        draw_row_rule(ui, palette);
    }
    copied
}

/// The results as labelled blocks, one per link, for windows too narrow for the table.
/// Returns the index of a row whose Copy button was clicked.
fn draw_results_stacked(
    ui: &mut egui::Ui,
    results: &[Resolution],
    palette: &Palette,
) -> Option<usize> {
    let mut copied = None;
    for (index, result) in results.iter().enumerate() {
        egui::Frame::new()
            .inner_margin(egui::Margin::symmetric(CARD_PADDING, CELL_PADDING_Y + 4))
            .show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                ui.spacing_mut().item_spacing.y = 4.0;
                ui.horizontal_wrapped(|ui| {
                    ui.label(format!("{}.", index + 1));
                    ui.label(status_text(result, palette));
                    if copy_button(ui, index, result, palette) {
                        copied = Some(index);
                    }
                });
                ui.label(secondary_text("Original link", palette));
                original_link(ui, result, palette);
                let label = if result.outcome.is_ok() {
                    "Destination"
                } else {
                    "Why it failed"
                };
                ui.label(secondary_text(label, palette));
                destination(ui, result);
            });
        draw_row_rule(ui, palette);
    }
    copied
}

/// "Resolved" in body text, or "Failed" in the problem colour, both bold.
fn status_text(result: &Resolution, palette: &Palette) -> RichText {
    let (label, color) = if result.outcome.is_ok() {
        ("Resolved", palette.text)
    } else {
        ("Failed", palette.problem)
    };
    RichText::new(label)
        .font(theme::bold(theme::BODY_SIZE))
        .color(color)
}

/// The original link on one line, cut short with an ellipsis when it does not fit. The full
/// text is the accessible name and the hover text; it is also in the text box and the CSV.
fn original_link(ui: &mut egui::Ui, result: &Resolution, palette: &Palette) {
    let response = ui.add(
        egui::Label::new(RichText::new(&result.input).color(palette.text_secondary)).truncate(),
    );
    let full_text = result.input.clone();
    ui.ctx()
        .accesskit_node_builder(response.id, |node| node.set_label(full_text));
}

/// The destination, always in full and wrapping; or why the link failed, in body text so
/// red stays a short label.
fn destination(ui: &mut egui::Ui, result: &Resolution) {
    let text = match &result.outcome {
        Ok(url) | Err(url) => url.as_str(),
    };
    ui.add(egui::Label::new(text).wrap().selectable(true));
}

/// A Copy button for a resolved row; nothing for a failed one. Returns whether it was clicked.
fn copy_button(ui: &mut egui::Ui, index: usize, result: &Resolution, palette: &Palette) -> bool {
    let Ok(url) = &result.outcome else {
        return false;
    };
    let number = index + 1;
    let clicked = secondary_button(ui, true, "Copy", palette)
        .on_hover_text(format!("Copy the destination of link {number}."))
        .clicked();
    if clicked {
        ui.ctx().copy_text(url.clone());
    }
    clicked
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::{Event, RawInput, Theme};

    fn app() -> RedirectApp {
        RedirectApp::new(&egui::Context::default(), None)
    }

    fn window(size: [f32; 2]) -> RawInput {
        RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size.into())),
            ..Default::default()
        }
    }

    fn run_frame(context: &egui::Context, app: &mut RedirectApp, input: RawInput) -> String {
        let mut output = context.run_ui(input, |ui| {
            egui::Frame::central_panel(ui.style())
                .inner_margin(egui::Margin::same(20))
                .show(ui, |ui| app.draw(ui));
        });
        // Headless tests inspect paint shapes but intentionally have no renderer to
        // upload texture deltas to.
        output.textures_delta.clear();
        output
            .shapes
            .iter()
            .filter_map(|shape| {
                if let egui::epaint::Shape::Text(text) = &shape.shape {
                    Some(text.galley.text())
                } else {
                    None
                }
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Text painted over several frames while the mouse wheel scrolls the page down, so
    /// controls below the fold in a small window count once they scroll into view. Each step
    /// is half the window height so consecutive frames overlap and no band is skipped.
    fn run_scrolling_frames(
        context: &egui::Context,
        app: &mut RedirectApp,
        size: [f32; 2],
    ) -> String {
        let mut seen = String::new();
        for _ in 0..40 {
            let mut input = window(size);
            input.events.push(Event::PointerMoved(egui::pos2(
                size[0] / 2.0,
                size[1] / 2.0,
            )));
            input.events.push(Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: egui::vec2(0.0, -size[1] / 2.0),
                modifiers: Modifiers::NONE,
                phase: egui::TouchPhase::Move,
                source: egui::MouseWheelSource::Wheel,
            });
            seen.push_str(&run_frame(context, app, input));
            seen.push('\n');
        }
        seen
    }

    #[test]
    fn empty_input_does_not_clear_previous_results_or_start_work() {
        let mut app = app();
        app.results.push(Resolution {
            input: "old".into(),
            outcome: Ok("https://example.com/".into()),
        });
        app.start_resolution(&egui::Context::default());
        assert!(app.worker.is_none());
        assert!(app.is_error);
        assert_eq!(app.results.len(), 1);
    }

    #[test]
    fn loading_updates_input_but_keeps_last_batch_and_cancel_keeps_input() {
        let mut app = app();
        app.input = "original".into();
        app.apply_event(WorkerEvent::Loaded(Ok(None)));
        assert_eq!(app.input, "original");
        app.apply_event(WorkerEvent::Loaded(Ok(Some(Loaded::Links(vec![
            "https://example.com/".into(),
        ])))));
        assert_eq!(app.input, "https://example.com/");
        assert!(app.status.contains("Click Resolve"));
    }

    fn table(text: &str) -> Table {
        Table::parse("links.csv", text, b',').unwrap()
    }

    #[test]
    fn a_table_with_one_link_column_loads_without_asking() {
        let mut app = app();
        app.apply_event(WorkerEvent::Loaded(Ok(Some(Loaded::Table(table(
            "Name,Website\nA,https://example.com/a\nB,\nC,www.example.org\n",
        ))))));
        assert!(app.column_choice.is_none());
        assert_eq!(app.input, "https://example.com/a\nwww.example.org");
        assert!(
            app.status.contains("from column \"Website\""),
            "{}",
            app.status
        );
        assert!(app.status.contains("1 row was blank"));
        assert!(!app.is_error);
    }

    #[test]
    fn a_table_with_several_link_columns_asks_and_uses_the_chosen_one() {
        let context = egui::Context::default();
        let mut app = RedirectApp::new(&context, None);
        run_frame(&context, &mut app, window([860.0, 680.0]));
        app.apply_event(WorkerEvent::Loaded(Ok(Some(Loaded::Table(table(
            "Source,Target\nhttps://a.com,https://b.com\nhttps://c.com,https://d.com\n",
        ))))));
        assert_eq!(app.column_choice.as_ref().map(|c| c.selected), Some(0));
        run_frame(&context, &mut app, window([860.0, 680.0]));
        let painted = run_frame(&context, &mut app, window([860.0, 680.0]));
        for text in [
            "Choose the column with links",
            "Several columns hold links",
            "First row is a header",
            "Source",
            "Target",
            "2 links",
            "https://a.com",
            "https://d.com",
            "Use column",
            "Cancel",
        ] {
            assert!(painted.contains(text), "missing {text}");
        }
        assert!(
            context.memory(|memory| memory.focused()).is_some(),
            "dialog did not take focus"
        );
        app.column_choice.as_mut().unwrap().selected = 1;
        app.use_column();
        assert!(app.column_choice.is_none());
        assert_eq!(app.input, "https://b.com\nhttps://d.com");
        assert!(app.status.contains("from column \"Target\""));
        assert_eq!(app.pending_focus, Some(FocusTarget::LoadButton));
        run_frame(&context, &mut app, window([860.0, 680.0]));
        assert_eq!(
            app.pending_focus, None,
            "focus was not handed back to Load file"
        );
    }

    #[test]
    fn a_table_without_link_columns_skips_and_flags_bad_rows_and_escape_cancels() {
        let context = egui::Context::default();
        let mut app = RedirectApp::new(&context, None);
        app.input = "kept".into();
        app.apply_event(WorkerEvent::Loaded(Ok(Some(Loaded::Table(table(
            "Note,Site\nhello,example.com\nworld,not a link\n",
        ))))));
        assert_eq!(app.column_choice.as_ref().map(|c| c.selected), Some(0));
        run_frame(&context, &mut app, window([860.0, 680.0]));
        let painted = run_frame(&context, &mut app, window([860.0, 680.0]));
        assert!(painted.contains("No column holds only links"));
        app.column_choice.as_mut().unwrap().selected = 1;
        app.use_column();
        assert_eq!(app.input, "example.com");
        assert!(
            app.status
                .contains("Skipped 1 row that do not contain a link: row 3"),
            "{}",
            app.status
        );
        assert!(app.is_error, "skipped rows must be flagged");

        app.input = "kept".into();
        app.apply_event(WorkerEvent::Loaded(Ok(Some(Loaded::Table(table(
            "Note,Site\nhello,example.com\nworld,not a link\n",
        ))))));
        run_frame(&context, &mut app, window([860.0, 680.0]));
        let mut input = window([860.0, 680.0]);
        input.events.push(Event::Key {
            key: Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        });
        run_frame(&context, &mut app, input);
        assert!(app.column_choice.is_none(), "Escape did not cancel");
        assert_eq!(app.input, "kept");
        assert!(app.status.contains("cancelled"));
    }

    #[test]
    fn completion_preserves_failures_and_copy_only_includes_successes() {
        let mut app = app();
        app.total = 2;
        app.apply_event(WorkerEvent::Resolved(Resolution {
            input: "bad".into(),
            outcome: Err("invalid".into()),
        }));
        app.apply_event(WorkerEvent::Resolved(Resolution {
            input: "good".into(),
            outcome: Ok("https://example.com/".into()),
        }));
        app.apply_event(WorkerEvent::Finished { cancelled: false });
        assert!(app.status.contains("1 resolved, 1 failed"));
        assert!(app.is_error);
        assert!(app.focus_results);
        assert_eq!(resolved_urls(&app.results), "https://example.com/");
    }

    #[test]
    fn unexpected_worker_exit_becomes_an_error_not_a_permanent_spinner() {
        let mut app = app();
        let (sender, receiver) = mpsc::channel();
        app.worker = Some(receiver);
        drop(sender);
        app.poll_worker();
        assert!(app.worker.is_none());
        assert!(app.is_error);
    }

    #[test]
    fn background_batch_reports_each_invalid_input_and_completes() {
        let (sender, receiver) = mpsc::channel();
        resolve_in_background(
            vec!["invalid".into(), "file:///not-allowed".into()],
            Arc::new(AtomicBool::new(false)),
            sender,
            egui::Context::default(),
        );
        for _ in 0..2 {
            assert!(matches!(
                receiver.recv().unwrap(),
                WorkerEvent::Resolved(Resolution {
                    outcome: Err(_),
                    ..
                })
            ));
        }
        assert!(matches!(
            receiver.recv().unwrap(),
            WorkerEvent::Finished { cancelled: false }
        ));
    }

    #[test]
    fn cancellation_prevents_starting_the_next_url() {
        let (sender, receiver) = mpsc::channel();
        resolve_in_background(
            vec!["invalid".into()],
            Arc::new(AtomicBool::new(true)),
            sender,
            egui::Context::default(),
        );
        assert!(matches!(
            receiver.recv().unwrap(),
            WorkerEvent::Finished { cancelled: true }
        ));
    }

    #[test]
    fn interface_renders_controls_and_results_in_both_themes_and_at_200_percent_zoom() {
        // 300 x 240 points is the 600 x 480 minimum window at 200 % zoom.
        for size in [[300.0, 240.0], [600.0, 480.0], [860.0, 680.0]] {
            for theme in [Theme::Light, Theme::Dark] {
                let context = egui::Context::default();
                context.set_theme(theme);
                let mut app = RedirectApp::new(&context, None);
                app.results.push(Resolution {
                    input: "https://example.com/start".into(),
                    outcome: Ok("https://example.com/final".into()),
                });
                app.results.push(Resolution {
                    input: "nonsense".into(),
                    outcome: Err("Enter a full link.".into()),
                });
                let text = run_scrolling_frames(&context, &mut app, size);
                for label in [
                    APP_NAME,
                    "Links to resolve",
                    "Load file…",
                    "Resolve",
                    "Results",
                    "2 links · 1 resolved · 1 failed",
                    "Save…",
                    "Copy resolved links",
                    "https://example.com/final",
                    "Failed",
                    "Enter a full link.",
                ] {
                    assert!(
                        text.contains(label),
                        "Missing {label} at {size:?} in {theme:?}: {text}"
                    );
                }
            }
        }
    }

    #[test]
    fn settings_dialog_opens_over_the_page_and_changes_are_saved_and_restored() {
        use crate::settings::ThemeChoice;
        let context = egui::Context::default();
        let mut app = RedirectApp::new(&context, None);
        let closed = run_frame(&context, &mut app, window([860.0, 680.0]));
        assert!(closed.contains("Settings"));
        assert!(!closed.contains("Follow system"));
        app.open_settings();
        // A new egui area spends its first frame on an invisible sizing pass.
        run_frame(&context, &mut app, window([860.0, 680.0]));
        let open = run_frame(&context, &mut app, window([860.0, 680.0]));
        for label in ["Follow system", "Text size", "Reduce motion", "Done"] {
            assert!(open.contains(label), "missing {label}");
        }
        // The page stays in place underneath rather than being pushed down.
        assert!(open.contains("Links to resolve") && open.contains("No results yet"));
        app.settings.theme = ThemeChoice::Dark;
        app.settings.zoom = 1.5;
        app.settings.apply(&context);
        run_frame(&context, &mut app, window([860.0, 680.0]));
        assert_eq!(context.theme(), egui::Theme::Dark);
        let mut storage = crate::settings::tests::MemoryStorage::default();
        eframe::App::save(&mut app, &mut storage);
        let restored = RedirectApp::new(&egui::Context::default(), Some(&storage));
        assert_eq!(restored.settings, app.settings);
    }

    #[test]
    fn settings_dialog_traps_focus_and_escape_returns_it_to_the_button() {
        let context = egui::Context::default();
        let mut app = RedirectApp::new(&context, None);
        run_frame(&context, &mut app, window([860.0, 680.0]));
        app.open_settings();
        run_frame(&context, &mut app, window([860.0, 680.0]));
        let in_dialog = context.memory(|memory| memory.focused());
        assert!(
            in_dialog.is_some(),
            "opening did not focus a dialog control"
        );
        // Losing focus (as a click on empty space would) is redirected back into the dialog.
        context.memory_mut(|memory| memory.surrender_focus(in_dialog.unwrap()));
        run_frame(&context, &mut app, window([860.0, 680.0]));
        assert_eq!(context.memory(|memory| memory.focused()), in_dialog);
        let mut input = window([860.0, 680.0]);
        input.events.push(Event::Key {
            key: Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        });
        run_frame(&context, &mut app, input);
        assert!(!app.settings_open, "Escape did not close the dialog");
        run_frame(&context, &mut app, window([860.0, 680.0]));
        let after = context.memory(|memory| memory.focused());
        assert!(
            after.is_some() && after != in_dialog,
            "focus did not return to the page"
        );
    }

    #[test]
    fn reduce_motion_hides_the_spinner() {
        let context = egui::Context::default();
        let mut app = RedirectApp::new(&context, None);
        let (_sender, receiver) = mpsc::channel();
        app.worker = Some(receiver);
        let count_spinner_arcs = |context: &egui::Context, app: &mut RedirectApp| {
            let mut output = context.run_ui(window([860.0, 680.0]), |ui| app.draw(ui));
            output.textures_delta.clear();
            output
                .shapes
                .iter()
                .filter(|shape| matches!(shape.shape, egui::epaint::Shape::Path(_)))
                .count()
        };
        let with_spinner = count_spinner_arcs(&context, &mut app);
        app.settings.reduce_motion = true;
        let without_spinner = count_spinner_arcs(&context, &mut app);
        assert!(
            with_spinner > without_spinner,
            "spinner still drawn with reduce motion"
        );
    }

    /// Painted text with the top-left corner of each piece, for checking where things sit.
    fn painted_positions(
        context: &egui::Context,
        app: &mut RedirectApp,
        size: [f32; 2],
    ) -> Vec<(String, egui::Pos2)> {
        let mut output = context.run_ui(window(size), |ui| {
            egui::Frame::central_panel(ui.style())
                .inner_margin(egui::Margin::same(20))
                .show(ui, |ui| app.draw(ui));
        });
        output.textures_delta.clear();
        output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::epaint::Shape::Text(text) => Some((text.galley.text().to_owned(), text.pos)),
                _ => None,
            })
            .collect()
    }

    fn position_of(painted: &[(String, egui::Pos2)], text: &str) -> egui::Pos2 {
        painted
            .iter()
            .find(|(painted_text, _)| painted_text == text)
            .unwrap_or_else(|| panic!("{text} was not painted"))
            .1
    }

    fn app_with_mixed_results(context: &egui::Context) -> RedirectApp {
        let mut app = RedirectApp::new(context, None);
        app.input = "https://example.com/start\nnonsense".into();
        app.results.push(Resolution {
            input: "https://example.com/start".into(),
            outcome: Ok("https://example.com/final".into()),
        });
        app.results.push(Resolution {
            input: "nonsense".into(),
            outcome: Err("Enter a full link.".into()),
        });
        app
    }

    #[test]
    fn results_actions_live_in_the_results_header_not_beside_resolve() {
        let context = egui::Context::default();
        let mut app = app_with_mixed_results(&context);
        let painted = painted_positions(&context, &mut app, [860.0, 680.0]);
        let resolve = position_of(&painted, "Resolve");
        let load = position_of(&painted, "Load file…");
        let title = position_of(&painted, "Results");
        let copy_all = position_of(&painted, "Copy resolved links");
        let save = position_of(&painted, "Save…");
        let first_destination = position_of(&painted, "https://example.com/final");
        // Load file and Resolve share the input card's row; the batch actions do not.
        assert!((load.y - resolve.y).abs() < 2.0);
        for action in [copy_all, save] {
            assert!(
                action.y > resolve.y + 20.0,
                "a results action is on the input row"
            );
            assert!(
                (action.y - title.y).abs() < 12.0,
                "a results action is not on the header bar"
            );
            assert!(
                action.y < first_destination.y,
                "a results action is below the table"
            );
        }
        assert!(copy_all.x < save.x, "Copy should read before Save");
    }

    #[test]
    fn wide_windows_show_a_table_and_narrow_ones_stack_each_link() {
        let context = egui::Context::default();
        let mut app = app_with_mixed_results(&context);
        let wide = run_frame(&context, &mut app, window([860.0, 680.0]));
        for heading in ["#", "Status", "Original link", "Destination"] {
            assert!(
                wide.lines().any(|line| line == heading),
                "missing {heading}"
            );
        }
        assert!(!wide.contains("Why it failed"));
        let narrow = run_scrolling_frames(&context, &mut app, [300.0, 240.0]);
        assert!(!narrow.lines().any(|line| line == "Status"));
        for label in ["1.", "Original link", "Destination", "Why it failed"] {
            assert!(narrow.contains(label), "missing {label} when stacked");
        }
    }

    #[test]
    fn destinations_wrap_in_full_and_original_links_keep_their_full_accessible_name() {
        let context = egui::Context::default();
        context.enable_accesskit();
        let mut app = RedirectApp::new(&context, None);
        let long_input = format!("https://example.com/{}", "source/".repeat(40));
        let long_destination = format!("https://example.org/{}", "final/".repeat(40));
        app.results.push(Resolution {
            input: long_input.clone(),
            outcome: Ok(long_destination.clone()),
        });
        // Narrow selects the stacked layout; tall keeps the result above the fold.
        for size in [[860.0, 680.0], [300.0, 2000.0]] {
            let mut output = context.run_ui(window(size), |ui| app.draw(ui));
            output.textures_delta.clear();
            // A galley keeps its full text when elided, so ask whether it was cut short.
            let elided = |wanted: &str| {
                output
                    .shapes
                    .iter()
                    .find_map(|shape| match &shape.shape {
                        egui::epaint::Shape::Text(text) if text.galley.text() == wanted => {
                            Some(text.galley.elided)
                        }
                        _ => None,
                    })
                    .unwrap_or_else(|| panic!("{wanted} was not painted at {size:?}"))
            };
            assert!(
                !elided(&long_destination),
                "destination was cut short at {size:?}"
            );
            assert!(
                elided(&long_input),
                "original link was expected to be truncated at {size:?}"
            );
            let update = output.platform_output.accesskit_update.unwrap();
            assert!(
                update
                    .nodes
                    .iter()
                    .any(|(_, node)| node.label() == Some(long_input.as_str())),
                "truncated original link lost its full accessible name at {size:?}"
            );
        }
    }

    #[test]
    fn status_shows_beside_what_caused_it_and_stays_one_live_region() {
        let context = egui::Context::default();
        context.enable_accesskit();
        let mut app = app_with_mixed_results(&context);
        let live_node = |context: &egui::Context, app: &mut RedirectApp| {
            let mut output = context.run_ui(window([860.0, 680.0]), |ui| app.draw(ui));
            output.textures_delta.clear();
            output
                .platform_output
                .accesskit_update
                .unwrap()
                .nodes
                .into_iter()
                .filter(|(_, node)| node.live() == Some(egui::accesskit::Live::Polite))
                .map(|(id, _)| id)
                .collect::<Vec<_>>()
        };

        app.set_status("Loaded 2 links. Click Resolve to start.", false);
        let painted = painted_positions(&context, &mut app, [860.0, 680.0]);
        assert!(
            position_of(&painted, "Loaded 2 links. Click Resolve to start.").y
                < position_of(&painted, "Results").y,
            "input status is not in the input card"
        );
        let input_region = live_node(&context, &mut app);

        app.apply_event(WorkerEvent::Finished { cancelled: false });
        let painted = painted_positions(&context, &mut app, [860.0, 680.0]);
        assert!(
            position_of(&painted, "Finished: 1 resolved, 1 failed.").y
                > position_of(&painted, "Results").y,
            "batch status is not in the results card"
        );
        let results_region = live_node(&context, &mut app);
        assert_eq!(input_region.len(), 1);
        assert_eq!(
            input_region, results_region,
            "the status line became a new live region when it moved"
        );
    }

    #[test]
    fn save_dialog_offers_both_exports_and_returns_focus_on_escape() {
        let context = egui::Context::default();
        let mut app = app_with_mixed_results(&context);
        run_frame(&context, &mut app, window([860.0, 680.0]));
        app.open_save_dialog();
        run_frame(&context, &mut app, window([860.0, 680.0]));
        let open = run_frame(&context, &mut app, window([860.0, 680.0]));
        for label in [
            "Save results",
            "Whole table (CSV)",
            "Resolved links only (text file)",
            "Failed links are left out.",
            "Cancel",
        ] {
            assert!(open.contains(label), "missing {label}");
        }
        assert!(context.memory(|memory| memory.focused()).is_some());
        app.export_format = ExportFormat::ResolvedLinks;
        let mut input = window([860.0, 680.0]);
        input.events.push(Event::Key {
            key: Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        });
        run_frame(&context, &mut app, input);
        assert!(!app.save_dialog_open, "Escape did not close the dialog");
        assert!(app.worker.is_none(), "closing started a save");
        assert_eq!(
            app.export_format,
            ExportFormat::ResolvedLinks,
            "choice forgotten"
        );
        run_frame(&context, &mut app, window([860.0, 680.0]));
        assert_eq!(app.pending_focus, None, "focus was not handed back to Save");
    }

    #[test]
    fn links_only_export_is_unavailable_when_nothing_resolved() {
        let context = egui::Context::default();
        let mut app = RedirectApp::new(&context, None);
        app.results.push(Resolution {
            input: "nonsense".into(),
            outcome: Err("Enter a full link.".into()),
        });
        app.export_format = ExportFormat::ResolvedLinks;
        app.open_save_dialog();
        assert_eq!(app.export_format, ExportFormat::Table);
        run_frame(&context, &mut app, window([860.0, 680.0]));
        let open = run_frame(&context, &mut app, window([860.0, 680.0]));
        assert!(open.contains("Not available: no link in this batch was resolved."));
    }

    #[test]
    fn column_widths_fill_the_table_and_favour_the_destination() {
        for width in [TABLE_MIN_WIDTH, 820.0, 1600.0] {
            let ColumnWidths(columns) = ColumnWidths::for_width(width);
            let total: f32 = columns.iter().sum();
            assert!(
                (total - width).abs() < 0.5,
                "{columns:?} do not add up to {width}"
            );
            assert!(
                columns[3] > columns[2],
                "destination narrower than original"
            );
            assert!(
                columns[2] > 100.0,
                "original link column too narrow at {width}"
            );
        }
    }

    #[test]
    fn ctrl_enter_starts_a_batch_from_the_text_box() {
        let context = egui::Context::default();
        let mut app = RedirectApp::new(&context, None);
        app.input = "invalid".into();
        let mut input = window([860.0, 680.0]);
        input.events.push(Event::Key {
            key: Key::Enter,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::COMMAND,
        });
        run_frame(&context, &mut app, input);
        assert!(app.worker.is_some(), "Ctrl+Enter did not start resolving");
        while let Some(receiver) = &app.worker {
            let event = receiver.recv_timeout(Duration::from_secs(5)).unwrap();
            app.apply_event(event);
        }
        assert_eq!(app.results.len(), 1);
        assert!(app.status.starts_with("Finished"));
    }

    #[test]
    fn plain_enter_only_edits_the_text_box() {
        let context = egui::Context::default();
        let mut app = RedirectApp::new(&context, None);
        app.input = "invalid".into();
        let mut input = window([860.0, 680.0]);
        input.events.push(Event::Key {
            key: Key::Enter,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        });
        run_frame(&context, &mut app, input);
        assert!(app.worker.is_none());
    }

    #[test]
    fn finishing_a_batch_moves_keyboard_focus_to_the_results_summary() {
        let context = egui::Context::default();
        let mut app = RedirectApp::new(&context, None);
        app.apply_event(WorkerEvent::Finished { cancelled: false });
        assert!(app.focus_results);
        run_frame(&context, &mut app, window([860.0, 680.0]));
        assert!(!app.focus_results);
        let focused = context.memory(|memory| memory.focused());
        assert!(focused.is_some(), "nothing received focus after completion");
        // A second frame keeps focus where it was put.
        run_frame(&context, &mut app, window([860.0, 680.0]));
        assert_eq!(context.memory(|memory| memory.focused()), focused);
    }

    #[test]
    fn status_line_is_a_polite_live_region_in_the_accessibility_tree() {
        let context = egui::Context::default();
        context.enable_accesskit();
        let mut app = RedirectApp::new(&context, None);
        app.set_status("Resolving 1 of 2 links…", false);
        let mut output = context.run_ui(window([860.0, 680.0]), |ui| app.draw(ui));
        output.textures_delta.clear();
        let update = output
            .platform_output
            .accesskit_update
            .expect("AccessKit tree was not produced");
        let live_status = update.nodes.iter().any(|(_, node)| {
            node.live() == Some(egui::accesskit::Live::Polite)
                && [node.label(), node.value()].contains(&Some("Resolving 1 of 2 links…"))
        });
        assert!(live_status, "status label is not a polite live region");
        let has_text_box_name = update.nodes.iter().any(|(_, node)| {
            node.role() == egui::accesskit::Role::MultilineTextInput
                && !node.labelled_by().is_empty()
        });
        assert!(has_text_box_name, "URL box has no accessible label");
    }

    #[test]
    fn ui_can_render_and_receive_progress_while_a_real_request_is_blocked() {
        use std::io::{Read, Write};
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/final", listener.local_addr().unwrap());
        let (seen_sender, seen_receiver) = mpsc::channel();
        let (release_sender, release_receiver) = mpsc::channel();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut request = Vec::new();
            let mut byte = [0];
            while !request.ends_with(b"\r\n\r\n") {
                assert_eq!(stream.read(&mut byte).unwrap(), 1);
                request.push(byte[0]);
            }
            seen_sender.send(()).unwrap();
            release_receiver
                .recv_timeout(Duration::from_secs(5))
                .unwrap();
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                .unwrap();
        });
        let context = egui::Context::default();
        let mut app = RedirectApp::new(&context, None);
        app.input = format!("invalid\n{url}\nshould-not-be-resolved");
        app.start_resolution(&context);
        seen_receiver.recv_timeout(Duration::from_secs(5)).unwrap();
        let text = run_frame(&context, &mut app, window([860.0, 680.0]));
        assert!(!text.is_empty());
        assert!(app.worker.is_some());
        assert_eq!(app.results.len(), 1);
        assert!(app.results[0].outcome.is_err());
        app.cancellation
            .as_ref()
            .unwrap()
            .store(true, Ordering::Relaxed);
        release_sender.send(()).unwrap();
        while let Some(receiver) = &app.worker {
            let event = receiver.recv_timeout(Duration::from_secs(5)).unwrap();
            app.apply_event(event);
        }
        assert_eq!(app.results[1].outcome.as_ref().unwrap(), &url);
        assert_eq!(app.results.len(), 2);
        assert!(app.status.contains("Stopped: 1 resolved, 1 failed"));
        server.join().unwrap();
    }
}
