//! Brand tokens, type scale and fonts shared by the light and dark themes.
//!
//! Colours come from the MSF USA brand book palette (pp. 42–48). The book does not define
//! interaction states or dark-theme surfaces, so each derived value below names its parent
//! colour. A unit test checks every text/background pair against the WCAG ratios the brand
//! book adopts: 4.5:1 for text, 3:1 for borders, focus rings and the progress fill.
//! Nothing in `gui.rs` names a colour, size or font directly; it asks this module.

use std::sync::Arc;

use eframe::egui::{
    self, Color32, FontData, FontDefinitions, FontFamily, FontId, Response, RichText, Stroke,
    StrokeKind, TextStyle, Theme,
};

// Brand palette (brand book p. 42 and p. 45).
const RED: Color32 = Color32::from_rgb(0xEE, 0x00, 0x00);
const LIGHT_RED: Color32 = Color32::from_rgb(0xF7, 0x80, 0x80);
const BLACK: Color32 = Color32::from_rgb(0x00, 0x00, 0x00);
const WHITE: Color32 = Color32::from_rgb(0xFF, 0xFF, 0xFF);
const DARK_GRAY: Color32 = Color32::from_rgb(0x3A, 0x3A, 0x3A);
const MEDIUM_GRAY: Color32 = Color32::from_rgb(0xD8, 0xD8, 0xD8);
const LIGHT_GRAY_2: Color32 = Color32::from_rgb(0xEF, 0xF0, 0xEB);

// Derived values the brand book does not define.
/// Brand Red darkened for the pressed primary button.
const RED_PRESSED: Color32 = Color32::from_rgb(0xC4, 0x00, 0x00);
/// Card and text-box surface on the dark theme, between Black and Dark Gray.
const DARK_SURFACE: Color32 = Color32::from_rgb(0x1E, 0x1E, 0x1E);
/// Hover fill on the dark theme, one step lighter than the surface.
const DARK_HOVER: Color32 = Color32::from_rgb(0x2A, 0x2A, 0x2A);
/// Card border and separator on the dark theme: the darkest grey that clears 3:1 on Black.
const DARK_BORDER: Color32 = Color32::from_rgb(0x60, 0x60, 0x60);
/// Control border on the dark theme: clears 3:1 on both Black and the dark surface.
const DARK_CONTROL_BORDER: Color32 = Color32::from_rgb(0x7A, 0x7A, 0x7A);
/// Selected-column tint in the light theme: Red at a whisper, so body text still clears AA.
const LIGHT_HIGHLIGHT: Color32 = Color32::from_rgb(0xFD, 0xE3, 0xE3);
/// Selected-column tint in the dark theme.
const DARK_HIGHLIGHT: Color32 = Color32::from_rgb(0x4A, 0x1F, 0x1F);

/// Type scale in points. Nothing user-facing is set below [`SECONDARY_SIZE`].
pub(crate) const HEADING_SIZE: f32 = 26.0;
pub(crate) const BODY_SIZE: f32 = 15.0;
pub(crate) const SECONDARY_SIZE: f32 = 13.0;

/// Width and gap of the keyboard focus ring drawn around controls.
pub(crate) const FOCUS_RING_WIDTH: f32 = 2.0;
pub(crate) const FOCUS_RING_GAP: f32 = 3.0;

/// Opacity of disabled controls: keeps Black text at 5.7:1 on white and White at 7.6:1 on Black.
const DISABLED_ALPHA: f32 = 0.6;

const MEDIUM_FONT: &str = "Ubuntu-Medium";
const BOLD_FONT: &str = "Ubuntu-Bold";
const BOLD_FAMILY: &str = "ubuntu-bold";

/// Colour roles resolved for one theme. Every pair a widget combines is tested in this module.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Palette {
    /// Window background: White or Black, the brand's two content backgrounds.
    pub background: Color32,
    /// Cards, the text box and secondary buttons.
    pub surface: Color32,
    /// Card borders and separators (decorative, not relied on to find a control).
    pub border: Color32,
    /// Button and text-box borders, which must clear 3:1 on their own.
    pub control_border: Color32,
    /// Body text.
    pub text: Color32,
    /// Helper lines, hint text, source links on cards.
    pub text_secondary: Color32,
    /// The heading rule, the primary button and the progress fill.
    pub accent: Color32,
    /// Primary button while pressed.
    pub accent_pressed: Color32,
    /// Text on the primary button.
    pub on_accent: Color32,
    /// The "Failed" label and the error status line. Never used for long text.
    pub problem: Color32,
    /// Keyboard focus ring on ordinary controls.
    pub focus_ring: Color32,
    /// Keyboard focus ring on the primary button, where red would vanish.
    pub focus_ring_on_accent: Color32,
    /// Secondary button while hovered.
    pub hover_fill: Color32,
    /// Secondary button while pressed or focused.
    pub pressed_fill: Color32,
    /// Progress bar track.
    pub track: Color32,
    /// Background of a selected column in a table preview. Always paired with another cue
    /// (a radio button), never the only sign of selection.
    pub highlight: Color32,
}

impl Palette {
    pub(crate) const LIGHT: Self = Self {
        background: WHITE,
        surface: WHITE,
        border: MEDIUM_GRAY,
        control_border: DARK_GRAY,
        text: BLACK,
        text_secondary: DARK_GRAY,
        accent: RED,
        accent_pressed: RED_PRESSED,
        on_accent: WHITE,
        problem: RED,
        focus_ring: RED,
        focus_ring_on_accent: BLACK,
        hover_fill: LIGHT_GRAY_2,
        pressed_fill: MEDIUM_GRAY,
        track: LIGHT_GRAY_2,
        highlight: LIGHT_HIGHLIGHT,
    };

    pub(crate) const DARK: Self = Self {
        background: BLACK,
        surface: DARK_SURFACE,
        border: DARK_BORDER,
        control_border: DARK_CONTROL_BORDER,
        text: WHITE,
        text_secondary: MEDIUM_GRAY,
        accent: RED,
        accent_pressed: RED_PRESSED,
        on_accent: WHITE,
        problem: LIGHT_RED,
        focus_ring: RED,
        focus_ring_on_accent: WHITE,
        hover_fill: DARK_HOVER,
        pressed_fill: DARK_GRAY,
        track: DARK_SURFACE,
        highlight: DARK_HIGHLIGHT,
    };

    /// The palette for the theme egui is currently rendering.
    pub(crate) fn for_theme(theme: Theme) -> Self {
        match theme {
            Theme::Light => Self::LIGHT,
            Theme::Dark => Self::DARK,
        }
    }
}

/// Bold Ubuntu at `size`, for headings, section labels and button text.
pub(crate) fn bold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(BOLD_FAMILY.into()))
}

/// Medium Ubuntu at the secondary size, for helper lines and source links.
pub(crate) fn secondary() -> FontId {
    FontId::new(SECONDARY_SIZE, FontFamily::Proportional)
}

/// Bold text in the body colour, for section labels and in-page headings.
pub(crate) fn bold_text(text: &str, size: f32, palette: &Palette) -> RichText {
    RichText::new(text).font(bold(size)).color(palette.text)
}

/// Secondary-size text in the secondary colour, for helper lines and source links.
pub(crate) fn secondary_text(text: &str, palette: &Palette) -> RichText {
    RichText::new(text)
        .font(secondary())
        .color(palette.text_secondary)
}

/// Register the brand fonts and apply the palette and type scale to both themes.
/// The window then follows the operating-system light/dark preference.
pub(crate) fn install(context: &egui::Context) {
    context.set_fonts(font_definitions());
    // When the operating system reports no preference, start light: the brand asks for white
    // as the content background whenever possible.
    context.options_mut(|options| options.fallback_theme = Theme::Light);
    for theme in [Theme::Light, Theme::Dark] {
        context.style_mut_of(theme, |style| apply(style, theme));
    }
}

/// Draw the keyboard focus ring around a control that has focus. Called after the control
/// is added so the ring is painted on top of it and only when the state is known.
pub(crate) fn focus_ring(ui: &egui::Ui, response: &Response, color: Color32) {
    if response.has_focus() {
        ui.painter().rect_stroke(
            response.rect.expand(FOCUS_RING_GAP),
            6.0,
            Stroke::new(FOCUS_RING_WIDTH, color),
            StrokeKind::Outside,
        );
    }
}

fn font_definitions() -> FontDefinitions {
    let mut fonts = FontDefinitions::default();
    fonts.font_data.insert(
        MEDIUM_FONT.to_owned(),
        Arc::new(FontData::from_static(include_bytes!(
            "../assets/fonts/Ubuntu-Medium.ttf"
        ))),
    );
    fonts.font_data.insert(
        BOLD_FONT.to_owned(),
        Arc::new(FontData::from_static(include_bytes!(
            "../assets/fonts/Ubuntu-Bold.ttf"
        ))),
    );
    // Medium replaces egui's bundled Ubuntu-Light everywhere it appears (first proportional
    // choice, and the glyph fallback for Monospace); the icon fallback after it stays.
    for family in fonts.families.values_mut() {
        for name in family.iter_mut() {
            if name == "Ubuntu-Light" {
                *name = MEDIUM_FONT.to_owned();
            }
        }
    }
    let proportional = fonts.families.entry(FontFamily::Proportional).or_default();
    if proportional.first().map(String::as_str) != Some(MEDIUM_FONT) {
        proportional.insert(0, MEDIUM_FONT.to_owned());
    }
    let mut bold_family = proportional.clone();
    bold_family[0] = BOLD_FONT.to_owned();
    fonts
        .families
        .insert(FontFamily::Name(BOLD_FAMILY.into()), bold_family);
    fonts
}

fn apply(style: &mut egui::Style, theme: Theme) {
    let palette = Palette::for_theme(theme);
    style.text_styles = [
        (TextStyle::Small, secondary()),
        (TextStyle::Body, FontId::proportional(BODY_SIZE)),
        (TextStyle::Button, bold(BODY_SIZE)),
        (TextStyle::Heading, bold(HEADING_SIZE)),
        (TextStyle::Monospace, FontId::monospace(BODY_SIZE)),
    ]
    .into();
    style.spacing.item_spacing = egui::vec2(8.0, 8.0);
    // 15 px text plus this padding gives every button a 33 px hit height.
    style.spacing.button_padding = egui::vec2(14.0, 9.0);

    let visuals = &mut style.visuals;
    visuals.override_text_color = Some(palette.text);
    visuals.weak_text_color = Some(palette.text_secondary);
    visuals.panel_fill = palette.background;
    visuals.window_fill = palette.background;
    visuals.extreme_bg_color = palette.track;
    visuals.text_edit_bg_color = Some(palette.surface);
    visuals.disabled_alpha = DISABLED_ALPHA;
    visuals.hyperlink_color = palette.accent;
    visuals.error_fg_color = palette.problem;
    visuals.warn_fg_color = palette.problem;
    visuals.selection.bg_fill = palette.pressed_fill;
    visuals.selection.stroke = Stroke::new(1.0, palette.text);
    visuals.text_cursor.stroke = Stroke::new(2.0, palette.text);

    let border = Stroke::new(1.0, palette.border);
    let control_border = Stroke::new(1.0, palette.control_border);
    let text = Stroke::new(1.0, palette.text);
    let widgets = &mut visuals.widgets;
    widgets.noninteractive.bg_fill = palette.background;
    widgets.noninteractive.weak_bg_fill = palette.background;
    widgets.noninteractive.bg_stroke = border;
    widgets.noninteractive.fg_stroke = text;
    for state in [
        &mut widgets.inactive,
        &mut widgets.hovered,
        &mut widgets.active,
        &mut widgets.open,
    ] {
        state.bg_stroke = control_border;
        state.fg_stroke = text;
        state.corner_radius = egui::CornerRadius::same(4);
    }
    widgets.inactive.bg_fill = palette.surface;
    widgets.inactive.weak_bg_fill = palette.surface;
    widgets.hovered.bg_fill = palette.hover_fill;
    widgets.hovered.weak_bg_fill = palette.hover_fill;
    widgets.open.bg_fill = palette.hover_fill;
    widgets.open.weak_bg_fill = palette.hover_fill;
    widgets.active.bg_fill = palette.pressed_fill;
    widgets.active.weak_bg_fill = palette.pressed_fill;
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEXT_MINIMUM: f64 = 4.5;
    const NON_TEXT_MINIMUM: f64 = 3.0;

    fn relative_luminance(color: Color32) -> f64 {
        let channel = |value: u8| {
            let value = f64::from(value) / 255.0;
            if value <= 0.03928 {
                value / 12.92
            } else {
                ((value + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * channel(color.r()) + 0.7152 * channel(color.g()) + 0.0722 * channel(color.b())
    }

    /// WCAG 2 contrast ratio, from 1 (identical) to 21 (black on white).
    fn contrast_ratio(a: Color32, b: Color32) -> f64 {
        let (first, second) = (relative_luminance(a), relative_luminance(b));
        (first.max(second) + 0.05) / (first.min(second) + 0.05)
    }

    fn assert_contrast(name: &str, foreground: Color32, background: Color32, minimum: f64) {
        let ratio = contrast_ratio(foreground, background);
        assert!(
            ratio >= minimum,
            "{name}: {foreground:?} on {background:?} is {ratio:.2}:1, needs {minimum}:1"
        );
    }

    #[test]
    fn every_palette_pair_meets_the_brand_book_contrast_chart() {
        for (theme, palette) in [("light", Palette::LIGHT), ("dark", Palette::DARK)] {
            let text_pairs = [
                ("body text on background", palette.text, palette.background),
                ("body text on surface", palette.text, palette.surface),
                ("body text on hover", palette.text, palette.hover_fill),
                ("body text on pressed", palette.text, palette.pressed_fill),
                ("body text on highlight", palette.text, palette.highlight),
                (
                    "secondary text on highlight",
                    palette.text_secondary,
                    palette.highlight,
                ),
                (
                    "secondary text on background",
                    palette.text_secondary,
                    palette.background,
                ),
                (
                    "secondary text on surface",
                    palette.text_secondary,
                    palette.surface,
                ),
                (
                    "problem text on background",
                    palette.problem,
                    palette.background,
                ),
                ("problem text on surface", palette.problem, palette.surface),
                ("button text on accent", palette.on_accent, palette.accent),
                (
                    "button text on pressed accent",
                    palette.on_accent,
                    palette.accent_pressed,
                ),
            ];
            for (name, foreground, background) in text_pairs {
                assert_contrast(
                    &format!("{theme}: {name}"),
                    foreground,
                    background,
                    TEXT_MINIMUM,
                );
            }
            let non_text_pairs = [
                (
                    "control border on background",
                    palette.control_border,
                    palette.background,
                ),
                (
                    "control border on surface",
                    palette.control_border,
                    palette.surface,
                ),
                (
                    "focus ring on background",
                    palette.focus_ring,
                    palette.background,
                ),
                ("focus ring on surface", palette.focus_ring, palette.surface),
                (
                    "focus ring on accent",
                    palette.focus_ring_on_accent,
                    palette.accent,
                ),
                ("progress fill on track", palette.accent, palette.track),
            ];
            for (name, foreground, background) in non_text_pairs {
                assert_contrast(
                    &format!("{theme}: {name}"),
                    foreground,
                    background,
                    NON_TEXT_MINIMUM,
                );
            }
        }
    }

    /// Disabled widgets are painted at [`DISABLED_ALPHA`] over the surface they sit on.
    #[test]
    fn disabled_text_stays_legible() {
        for palette in [Palette::LIGHT, Palette::DARK] {
            let blended = Color32::from_rgb(
                blend(palette.text.r(), palette.surface.r()),
                blend(palette.text.g(), palette.surface.g()),
                blend(palette.text.b(), palette.surface.b()),
            );
            assert_contrast(
                "disabled text on surface",
                blended,
                palette.surface,
                TEXT_MINIMUM,
            );
        }
    }

    fn blend(foreground: u8, background: u8) -> u8 {
        let alpha = f64::from(DISABLED_ALPHA);
        (f64::from(foreground) * alpha + f64::from(background) * (1.0 - alpha)).round() as u8
    }

    #[test]
    fn no_text_style_falls_below_the_secondary_size() {
        for theme in [Theme::Light, Theme::Dark] {
            let mut style = egui::Style::default();
            apply(&mut style, theme);
            for (text_style, font) in &style.text_styles {
                assert!(
                    font.size >= SECONDARY_SIZE,
                    "{text_style:?} is {} px",
                    font.size
                );
            }
            assert_eq!(style.text_styles[&TextStyle::Heading].size, HEADING_SIZE);
            assert_eq!(
                style.text_styles[&TextStyle::Button].family,
                bold(1.0).family
            );
        }
    }

    #[test]
    fn fonts_replace_light_weight_and_keep_fallbacks() {
        let fonts = font_definitions();
        let proportional = &fonts.families[&FontFamily::Proportional];
        assert_eq!(proportional[0], MEDIUM_FONT);
        assert!(proportional.iter().any(|name| name == "egui-icons"));
        for (family, names) in &fonts.families {
            assert!(
                !names.iter().any(|name| name == "Ubuntu-Light"),
                "{family:?} still falls back to the light weight"
            );
        }
        let bold_family = &fonts.families[&FontFamily::Name(BOLD_FAMILY.into())];
        assert_eq!(bold_family[0], BOLD_FONT);
        assert_eq!(&bold_family[1..], &proportional[1..]);
        // Parse both files so a corrupt asset fails here rather than at first paint.
        let context = egui::Context::default();
        install(&context);
        let mut output = context.run_ui(Default::default(), |ui| {
            ui.heading("Link Resolver");
            ui.label("body");
        });
        // Headless: there is no renderer to receive the font atlas.
        output.textures_delta.clear();
        assert!(!output.shapes.is_empty());
    }
}
