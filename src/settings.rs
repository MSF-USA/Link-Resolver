//! User preferences: theme, text size and motion.
//!
//! Preferences are the single source of truth for the window's zoom and theme. They are
//! stored as plain `key = value` lines through eframe's storage so they survive restarts
//! without a serialisation dependency, and the keyboard zoom shortcuts are handled here so
//! the stored value never drifts from what the window shows.

use eframe::egui::{self, Id, ThemePreference};

use crate::theme::{self, Palette};

/// Storage key under which the settings text is kept.
const STORAGE_KEY: &str = "settings";

/// Zoom range offered by the UI and accepted from storage. 3.0 covers the WCAG 200 % target
/// with headroom; below 0.75 the type scale drops under its readable floor.
pub(crate) const MIN_ZOOM: f32 = 0.75;
pub(crate) const MAX_ZOOM: f32 = 3.0;
const ZOOM_STEP: f32 = 0.1;
const ZOOM_EPSILON: f32 = 0.001;

/// Which theme the window uses.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum ThemeChoice {
    /// Follow the operating system, falling back to light.
    #[default]
    System,
    Light,
    Dark,
}

impl ThemeChoice {
    fn preference(self) -> ThemePreference {
        match self {
            Self::System => ThemePreference::System,
            Self::Light => ThemePreference::Light,
            Self::Dark => ThemePreference::Dark,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::Light => "light",
            Self::Dark => "dark",
        }
    }

    fn from_name(name: &str) -> Option<Self> {
        match name {
            "system" => Some(Self::System),
            "light" => Some(Self::Light),
            "dark" => Some(Self::Dark),
            _ => None,
        }
    }
}

/// Everything the user can change in the Settings panel.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Settings {
    pub theme: ThemeChoice,
    /// Scale applied to the whole window; 1.0 is the designed size.
    pub zoom: f32,
    /// Hide the animated spinner and disable widget animations.
    pub reduce_motion: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: ThemeChoice::System,
            zoom: 1.0,
            reduce_motion: false,
        }
    }
}

impl Settings {
    /// Read saved settings, or the defaults when there is no storage or nothing saved yet.
    pub(crate) fn load(storage: Option<&dyn eframe::Storage>) -> Self {
        storage
            .and_then(|storage| storage.get_string(STORAGE_KEY))
            .map(|text| Self::parse(&text))
            .unwrap_or_default()
    }

    /// Write the settings so the next launch restores them.
    pub(crate) fn save(&self, storage: &mut dyn eframe::Storage) {
        storage.set_string(STORAGE_KEY, self.to_text());
    }

    /// Parse `key = value` lines. Unknown keys are ignored and unreadable or out-of-range
    /// values fall back to the default, so an edited or outdated file never blocks startup.
    pub(crate) fn parse(text: &str) -> Self {
        let mut settings = Self::default();
        for line in text.lines() {
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let value = value.trim();
            match key.trim() {
                "theme" => settings.theme = ThemeChoice::from_name(value).unwrap_or_default(),
                "zoom" => {
                    settings.zoom = value
                        .parse::<f32>()
                        .ok()
                        .filter(|zoom| zoom.is_finite())
                        .map_or(1.0, |zoom| zoom.clamp(MIN_ZOOM, MAX_ZOOM));
                }
                "reduce_motion" => settings.reduce_motion = value == "true",
                _ => {}
            }
        }
        settings
    }

    /// The text form read back by [`Self::parse`].
    pub(crate) fn to_text(&self) -> String {
        format!(
            "theme = {}\nzoom = {:.2}\nreduce_motion = {}\n",
            self.theme.name(),
            self.zoom,
            self.reduce_motion
        )
    }

    /// Push the settings into the context. Zoom takes effect on the next frame.
    pub(crate) fn apply(&self, context: &egui::Context) {
        context.set_theme(self.theme.preference());
        context.set_zoom_factor(self.zoom);
        // The shortcuts are handled by `handle_shortcuts` so this struct stays the source of truth.
        context.options_mut(|options| options.zoom_with_keyboard = false);
        let animation_time = if self.reduce_motion {
            0.0
        } else {
            egui::Style::default().animation_time
        };
        context.all_styles_mut(|style| style.animation_time = animation_time);
        context.request_repaint();
    }

    /// Consume Ctrl +, Ctrl − and Ctrl 0 (Cmd on macOS). Returns true when the zoom changed,
    /// in which case the caller applies and saves.
    pub(crate) fn handle_shortcuts(&mut self, context: &egui::Context) -> bool {
        use egui::gui_zoom::kb_shortcuts::{ZOOM_IN, ZOOM_IN_SECONDARY, ZOOM_OUT, ZOOM_RESET};
        let before = self.zoom;
        context.input_mut(|input| {
            if input.consume_shortcut(&ZOOM_RESET) {
                self.zoom = 1.0;
            } else if input.consume_shortcut(&ZOOM_IN) || input.consume_shortcut(&ZOOM_IN_SECONDARY)
            {
                self.step_zoom(ZOOM_STEP);
            } else if input.consume_shortcut(&ZOOM_OUT) {
                self.step_zoom(-ZOOM_STEP);
            }
        });
        (self.zoom - before).abs() > ZOOM_EPSILON
    }

    fn step_zoom(&mut self, delta: f32) {
        self.zoom = ((self.zoom + delta) * 100.0).round() / 100.0;
        self.zoom = self.zoom.clamp(MIN_ZOOM, MAX_ZOOM);
    }

    /// Draw the settings controls. Returns true when a value changed, in which case the
    /// caller applies and saves. The id of every focusable control is appended to
    /// `controls`, in Tab order, so a dialog can keep keyboard focus inside itself.
    pub(crate) fn draw(
        &mut self,
        ui: &mut egui::Ui,
        palette: &Palette,
        controls: &mut Vec<Id>,
    ) -> bool {
        let before = self.clone();

        ui.label(theme::bold_text("Theme", theme::BODY_SIZE, palette));
        ui.horizontal_wrapped(|ui| {
            for (choice, label) in [
                (ThemeChoice::System, "Follow system"),
                (ThemeChoice::Light, "Light"),
                (ThemeChoice::Dark, "Dark"),
            ] {
                let response = ui.radio_value(&mut self.theme, choice, label);
                theme::focus_ring(ui, &response, palette.focus_ring);
                controls.push(response.id);
            }
        });
        ui.add_space(12.0);

        ui.label(theme::bold_text("Text size", theme::BODY_SIZE, palette));
        ui.horizontal_wrapped(|ui| {
            let smaller = ui.add_enabled(
                self.zoom > MIN_ZOOM + ZOOM_EPSILON,
                egui::Button::new("Smaller"),
            );
            theme::focus_ring(ui, &smaller, palette.focus_ring);
            controls.push(smaller.id);
            if smaller.clicked() {
                self.step_zoom(-ZOOM_STEP);
            }
            let larger = ui.add_enabled(
                self.zoom < MAX_ZOOM - ZOOM_EPSILON,
                egui::Button::new("Larger"),
            );
            theme::focus_ring(ui, &larger, palette.focus_ring);
            controls.push(larger.id);
            if larger.clicked() {
                self.step_zoom(ZOOM_STEP);
            }
            let reset = ui.add_enabled(
                (self.zoom - 1.0).abs() > ZOOM_EPSILON,
                egui::Button::new("Reset"),
            );
            theme::focus_ring(ui, &reset, palette.focus_ring);
            controls.push(reset.id);
            if reset.clicked() {
                self.zoom = 1.0;
            }
            // The value comes last so the buttons never shift as the digits change.
            let percent = (self.zoom * 100.0).round() as i32;
            ui.label(theme::bold_text(
                &format!("{percent} %"),
                theme::BODY_SIZE,
                palette,
            ));
        });
        ui.label(theme::secondary_text(
            "Ctrl and + or - also change the size. Ctrl and 0 resets it.",
            palette,
        ));
        ui.add_space(12.0);

        let response = ui.checkbox(&mut self.reduce_motion, "Reduce motion");
        theme::focus_ring(ui, &response, palette.focus_ring);
        controls.push(response.id);
        ui.label(theme::secondary_text(
            "Hides the spinning indicator while links resolve. Progress is still shown as text.",
            palette,
        ));

        *self != before
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::collections::BTreeMap;

    /// In-memory stand-in for eframe's file storage.
    #[derive(Default)]
    pub(crate) struct MemoryStorage(BTreeMap<String, String>);

    impl eframe::Storage for MemoryStorage {
        fn get_string(&self, key: &str) -> Option<String> {
            self.0.get(key).cloned()
        }
        fn set_string(&mut self, key: &str, value: String) {
            self.0.insert(key.to_owned(), value);
        }
        fn remove_string(&mut self, key: &str) {
            self.0.remove(key);
        }
        fn flush(&mut self) {}
    }

    #[test]
    fn settings_round_trip_through_storage() {
        let settings = Settings {
            theme: ThemeChoice::Dark,
            zoom: 1.5,
            reduce_motion: true,
        };
        let mut storage = MemoryStorage::default();
        settings.save(&mut storage);
        assert_eq!(Settings::load(Some(&storage)), settings);
        assert_eq!(Settings::load(None), Settings::default());
    }

    #[test]
    fn parsing_tolerates_garbage_unknown_keys_and_out_of_range_values() {
        let parsed = Settings::parse("not a setting\nzoom = 9\ntheme = purple\nfuture = x\n");
        assert_eq!(parsed.zoom, MAX_ZOOM);
        assert_eq!(parsed.theme, ThemeChoice::System);
        assert!(!parsed.reduce_motion);
        assert_eq!(Settings::parse("zoom = nan").zoom, 1.0);
        assert_eq!(Settings::parse("zoom = 0.1").zoom, MIN_ZOOM);
        assert_eq!(Settings::parse("").zoom, 1.0);
        assert!(Settings::parse("reduce_motion = true").reduce_motion);
    }

    #[test]
    fn applying_sets_theme_zoom_and_animation() {
        let context = egui::Context::default();
        let settings = Settings {
            theme: ThemeChoice::Light,
            zoom: 1.3,
            reduce_motion: true,
        };
        settings.apply(&context);
        // Zoom is picked up at the start of the next frame.
        let mut output = context.run_ui(Default::default(), |ui| {
            ui.label("frame");
        });
        output.textures_delta.clear();
        assert_eq!(context.theme(), egui::Theme::Light);
        assert!((context.zoom_factor() - 1.3).abs() < ZOOM_EPSILON);
        assert_eq!(context.global_style().animation_time, 0.0);
        assert!(!context.options(|options| options.zoom_with_keyboard));

        Settings::default().apply(&context);
        assert!(context.global_style().animation_time > 0.0);
    }

    #[test]
    fn zoom_shortcuts_step_clamp_and_reset() {
        let context = egui::Context::default();
        let mut settings = Settings {
            zoom: MAX_ZOOM - 0.05,
            ..Settings::default()
        };
        /// Run one frame with Ctrl+`key` pressed and report whether the zoom changed.
        fn press(context: &egui::Context, settings: &mut Settings, key: egui::Key) -> bool {
            let mut input = egui::RawInput::default();
            input.events.push(egui::Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::COMMAND,
            });
            let mut changed = false;
            let mut output = context.run_ui(input, |ui| {
                changed = settings.handle_shortcuts(ui.ctx());
            });
            output.textures_delta.clear();
            changed
        }
        assert!(press(&context, &mut settings, egui::Key::Plus));
        assert_eq!(settings.zoom, MAX_ZOOM);
        assert!(
            !press(&context, &mut settings, egui::Key::Plus),
            "zoom exceeded the maximum"
        );
        assert!(press(&context, &mut settings, egui::Key::Minus));
        assert!((settings.zoom - (MAX_ZOOM - 0.1)).abs() < ZOOM_EPSILON);
        assert!(press(&context, &mut settings, egui::Key::Num0));
        assert_eq!(settings.zoom, 1.0);
        assert!(!press(&context, &mut settings, egui::Key::Num0));
    }

    #[test]
    fn panel_reports_changes_and_renders_every_control() {
        let context = egui::Context::default();
        let mut settings = Settings::default();
        let mut changed = None;
        let mut controls = Vec::new();
        let mut output = context.run_ui(Default::default(), |ui| {
            changed = Some(settings.draw(ui, &Palette::LIGHT, &mut controls));
        });
        output.textures_delta.clear();
        assert_eq!(changed, Some(false));
        // Three radios, three buttons and one checkbox, all focusable and distinct.
        assert_eq!(controls.len(), 7);
        assert_eq!(
            controls
                .iter()
                .collect::<std::collections::HashSet<_>>()
                .len(),
            7
        );
        let text = output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::epaint::Shape::Text(text) => Some(text.galley.text()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");
        for label in [
            "Theme",
            "Follow system",
            "Light",
            "Dark",
            "Text size",
            "100 %",
            "Smaller",
            "Larger",
            "Reset",
            "Reduce motion",
        ] {
            assert!(text.contains(label), "missing {label}: {text}");
        }
    }
}
