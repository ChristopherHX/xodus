//! Xodus Design System — egui stylesheet
//! ---------------------------------------------------------------------------
//! Translated directly from the Xodus design tokens (colors / spacing /
//! effects / typography). Ships both themes: `xodus_dark()` (the default
//! surface for the gamepad session) and `xodus_light()` (desktop windows).
//!
//! Tested against egui 0.28 / 0.29. Notes for other versions:
//!   • egui >= 0.31 renamed `CornerRadius` -> `CornerRadius` and
//!     `CornerRadius::same(x)` takes `u8`. Swap the type alias below if needed.
//!   • Fonts require the .ttf files — see `install_fonts()`.
//!
//! Usage:
//! ```no_run
//! // once, at startup (after creating the egui context):
//! xodus_theme::install_fonts(&ctx);          // optional, needs font files
//! ctx.set_style(xodus_theme::xodus_dark());   // or xodus_light()
//! ```

use std::sync::Arc;

use egui::{
    Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Stroke, Style, TextStyle,
    Vec2, Visuals,
    epaint::Shadow,
    style::{Selection, WidgetVisuals, Widgets},
};

/* ============================================================
COLOR PRIMITIVES  (from tokens/colors.css)
============================================================ */

// Brand green ramp
pub const GREEN_300: Color32 = Color32::from_rgb(0x70, 0xF0, 0x30); // lime highlight / glow
pub const GREEN_400: Color32 = Color32::from_rgb(0x2F, 0xCB, 0x1F);
pub const GREEN_500: Color32 = Color32::from_rgb(0x00, 0xB0, 0x00); // PRIMARY brand green
pub const GREEN_600: Color32 = Color32::from_rgb(0x00, 0x92, 0x00); // hover
pub const GREEN_700: Color32 = Color32::from_rgb(0x00, 0x7A, 0x00); // press / deep
pub const GREEN_900: Color32 = Color32::from_rgb(0x0B, 0x3D, 0x0B); // green-tinted ink

// Tux accents + ink
pub const ORANGE: Color32 = Color32::from_rgb(0xF7, 0xA8, 0x1B);
pub const ORANGE_HI: Color32 = Color32::from_rgb(0xFF, 0xC2, 0x4D);
pub const INK: Color32 = Color32::from_rgb(0x14, 0x14, 0x14);
pub const WHITE: Color32 = Color32::from_rgb(0xFF, 0xFF, 0xFF);

// Semantic status
pub const STATUS_SUCCESS: Color32 = GREEN_500;
pub const STATUS_WARNING: Color32 = ORANGE;
pub const STATUS_DANGER: Color32 = Color32::from_rgb(0xE5, 0x48, 0x4D);
pub const STATUS_INFO: Color32 = Color32::from_rgb(0x3B, 0x9E, 0xFF);

// Neutral ramp
pub const GRAY_50: Color32 = Color32::from_rgb(0xF7, 0xF8, 0xFA);
pub const GRAY_100: Color32 = Color32::from_rgb(0xF1, 0xF3, 0xF5);
pub const GRAY_200: Color32 = Color32::from_rgb(0xE4, 0xE7, 0xEB);
pub const GRAY_300: Color32 = Color32::from_rgb(0xD2, 0xD7, 0xDD);
pub const GRAY_400: Color32 = Color32::from_rgb(0xA8, 0xB0, 0xB9);
pub const GRAY_500: Color32 = Color32::from_rgb(0x7C, 0x84, 0x8E);
pub const GRAY_600: Color32 = Color32::from_rgb(0x5A, 0x61, 0x6B);
pub const GRAY_700: Color32 = Color32::from_rgb(0x3B, 0x42, 0x4B);
pub const GRAY_800: Color32 = Color32::from_rgb(0x22, 0x27, 0x2D);
pub const GRAY_900: Color32 = Color32::from_rgb(0x16, 0x1A, 0x1E);
pub const GRAY_950: Color32 = Color32::from_rgb(0x0E, 0x11, 0x14);

/* ============================================================
SPACING  (tokens/spacing.css — 4px base grid)
============================================================ */
pub const SPACE_1: f32 = 4.0;
pub const SPACE_2: f32 = 8.0;
pub const SPACE_3: f32 = 12.0;
pub const SPACE_4: f32 = 16.0;
pub const SPACE_5: f32 = 20.0;
pub const SPACE_6: f32 = 24.0;
pub const SPACE_8: f32 = 32.0;

// Control heights — gamepad hit targets never below 44px
pub const CONTROL_H_SM: f32 = 34.0;
pub const CONTROL_H_MD: f32 = 44.0; // default
pub const CONTROL_H_LG: f32 = 56.0;

/* ============================================================
RADII  (tokens/effects.css)
egui uses one radius per widget; these are the token values.
============================================================ */
pub const RADIUS_XS: f32 = 6.0;
pub const RADIUS_SM: f32 = 8.0;
pub const RADIUS_MD: f32 = 12.0; // buttons / inputs
pub const RADIUS_LG: f32 = 16.0; // cards
pub const RADIUS_XL: f32 = 22.0; // game tiles
pub const RADIUS_2XL: f32 = 28.0;

/* ============================================================
FONTS  (tokens/typography.css)
Display = Space Grotesk · Body/UI = Manrope · Mono = JetBrains Mono
============================================================ */

/// Register the Xodus font families. Requires the .ttf files on disk.
/// Adjust the paths to wherever you vendor the fonts.
pub fn install_fonts(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();

    // --- load font bytes (edit paths / use include_bytes! as you prefer) ---
    fonts.font_data.insert(
        "Manrope".to_owned(),
        Arc::new(FontData::from_static(include_bytes!(
            "../assets/fonts/Manrope-Regular.ttf"
        ))),
    );
    fonts.font_data.insert(
        "SpaceGrotesk".to_owned(),
        Arc::new(FontData::from_static(include_bytes!(
            "../assets/fonts/Manrope-Regular.ttf"
        ))),
    );
    fonts.font_data.insert(
        "JetBrainsMono".to_owned(),
        Arc::new(FontData::from_static(include_bytes!(
            "../assets/fonts/JetBrainsMono-Regular.ttf"
        ))),
    );

    // Manrope is the default proportional UI face.
    fonts
        .families
        .entry(FontFamily::Proportional)
        .or_default()
        .insert(0, "Manrope".to_owned());
    // JetBrains Mono for paths / versions / metrics.
    fonts
        .families
        .entry(FontFamily::Monospace)
        .or_default()
        .insert(0, "JetBrainsMono".to_owned());

    // Space Grotesk as a named family for display headings.
    // Use it with FontId::new(size, FontFamily::Name("Display".into())).
    fonts.families.insert(
        FontFamily::Name("Display".into()),
        vec!["SpaceGrotesk".to_owned()],
    );

    ctx.set_fonts(fonts);
}

/// Type scale wired to egui's `TextStyle`s (px sizes from the token scale).
/// Body = Manrope 15px; headings = Space Grotesk display family.
fn install_text_styles(style: &mut Style) {
    use FontFamily::{Monospace, Proportional};
    let display = FontFamily::Name("Display".into());
    style.text_styles = [
        (TextStyle::Small, FontId::new(12.0, Proportional)), // --text-xs
        (TextStyle::Body, FontId::new(15.0, Proportional)),  // --text-base
        (TextStyle::Button, FontId::new(15.0, Proportional)),
        (TextStyle::Monospace, FontId::new(13.0, Monospace)), // paths/versions
        (TextStyle::Heading, FontId::new(24.0, display)),     // --text-xl display
    ]
    .into();
}

/* ============================================================
SPACING / STYLE  (shared between themes)
============================================================ */
fn install_spacing(style: &mut Style) {
    let s = &mut style.spacing;
    s.item_spacing = Vec2::new(SPACE_2, SPACE_2); // 8px gaps
    s.button_padding = Vec2::new(SPACE_4, SPACE_3); // 16 / 12
    s.menu_margin = egui::Margin::same(SPACE_2 as _);
    s.window_margin = egui::Margin::same(SPACE_5 as _);
    s.indent = SPACE_5;
    s.interact_size = Vec2::new(0.0, CONTROL_H_MD); // 44px default control height
    s.icon_width = 20.0; // 16/20/24 icon rhythm
    s.icon_width_inner = 14.0;
    s.icon_spacing = SPACE_2;
    s.scroll = egui::style::ScrollStyle::solid();
}

// Soft, low-contrast console elevation (Switch-like).
fn shadow_sm(dark: bool) -> Shadow {
    Shadow {
        offset: [0, 2],
        blur: if dark { 8 } else { 6 },
        spread: 0,
        color: if dark {
            Color32::from_black_alpha(115) // rgba(0,0,0,.45)
        } else {
            Color32::from_rgba_unmultiplied(16, 24, 32, 20) // rgba(16,24,32,.08)
        },
    }
}

fn shadow_lg(dark: bool) -> Shadow {
    Shadow {
        offset: [0, 16],
        blur: if dark { 48 } else { 40 },
        spread: 0,
        color: if dark {
            Color32::from_black_alpha(153) // .6
        } else {
            Color32::from_rgba_unmultiplied(16, 24, 32, 41) // .16
        },
    }
}

/* ============================================================
DARK THEME  — default surface for the gamepad session
============================================================ */
pub fn xodus_dark() -> Style {
    // token values
    let bg_app = GRAY_950; // #0E1114
    let surface = Color32::from_rgb(0x18, 0x1D, 0x22);
    let surface_2 = Color32::from_rgb(0x12, 0x16, 0x1A);
    let surface_raised = Color32::from_rgb(0x1F, 0x25, 0x2B);
    let border = Color32::from_rgb(0x2A, 0x31, 0x38);
    let border_strong = Color32::from_rgb(0x3A, 0x42, 0x4B);
    let text_primary = Color32::from_rgb(0xF2, 0xF5, 0xF7);
    let text_secondary = Color32::from_rgb(0xAE, 0xB6, 0xBF);
    let text_muted = Color32::from_rgb(0x6E, 0x76, 0x7F);

    let mut v = Visuals::dark();
    v.dark_mode = true;
    v.override_text_color = Some(text_primary);
    v.panel_fill = bg_app;
    v.window_fill = surface;
    v.window_stroke = Stroke::new(1.0, border);
    v.window_corner_radius = CornerRadius::same(RADIUS_LG as u8); // 16px cards/windows
    v.window_shadow = shadow_lg(true);
    v.popup_shadow = shadow_sm(true);
    v.extreme_bg_color = surface_2; // text-edit / recessed backgrounds
    v.faint_bg_color = surface_2;
    v.code_bg_color = surface_2;
    v.hyperlink_color = GREEN_400;
    v.warn_fg_color = ORANGE;
    v.error_fg_color = STATUS_DANGER;

    // Signature green selection / focus ring.
    v.selection = Selection {
        bg_fill: GREEN_500.linear_multiply(0.32), // --brand-soft-ish
        stroke: Stroke::new(1.0, GREEN_400),
    };

    v.widgets = Widgets {
        // Non-interactive text / separators.
        noninteractive: WidgetVisuals {
            bg_fill: bg_app,
            weak_bg_fill: bg_app,
            bg_stroke: Stroke::new(1.0, border),
            fg_stroke: Stroke::new(1.0, text_secondary),
            corner_radius: CornerRadius::same(RADIUS_MD as u8),
            expansion: 0.0,
        },
        // Idle interactive controls (secondary button look).
        inactive: WidgetVisuals {
            bg_fill: surface_raised,
            weak_bg_fill: surface_raised,
            bg_stroke: Stroke::new(1.0, border_strong),
            fg_stroke: Stroke::new(1.0, text_primary),
            corner_radius: CornerRadius::same(RADIUS_MD as u8),
            expansion: 0.0,
        },
        // Hover — lift toward brand green.
        hovered: WidgetVisuals {
            bg_fill: GREEN_600,
            weak_bg_fill: GREEN_500.linear_multiply(0.16),
            bg_stroke: Stroke::new(1.0, GREEN_500),
            fg_stroke: Stroke::new(1.5, text_primary),
            corner_radius: CornerRadius::same(RADIUS_MD as u8),
            expansion: 1.0,
        },
        // Active / pressed.
        active: WidgetVisuals {
            bg_fill: GREEN_500,
            weak_bg_fill: GREEN_500,
            bg_stroke: Stroke::new(1.0, GREEN_600),
            fg_stroke: Stroke::new(2.0, Color32::from_rgb(0x0A, 0x1F, 0x0A)), // text-on-brand
            corner_radius: CornerRadius::same(RADIUS_MD as u8),
            expansion: 0.0,
        },
        // Open menus / combo boxes.
        open: WidgetVisuals {
            bg_fill: surface_raised,
            weak_bg_fill: surface_raised,
            bg_stroke: Stroke::new(1.0, GREEN_500),
            fg_stroke: Stroke::new(1.0, text_primary),
            corner_radius: CornerRadius::same(RADIUS_MD as u8),
            expansion: 0.0,
        },
    };

    let _ = text_muted; // available for custom widgets
    let mut style = Style {
        visuals: v,
        ..Default::default()
    };
    install_spacing(&mut style);
    install_text_styles(&mut style);
    style
}

/* ============================================================
LIGHT THEME  — desktop windows
============================================================ */
pub fn xodus_light() -> Style {
    let bg_app = Color32::from_rgb(0xEE, 0xF0, 0xF3);
    let surface = WHITE;
    let surface_2 = GRAY_50;
    let border = Color32::from_rgb(0xE2, 0xE5, 0xEA);
    let border_strong = Color32::from_rgb(0xCD, 0xD2, 0xD9);
    let text_primary = Color32::from_rgb(0x18, 0x1B, 0x1F);
    let text_secondary = Color32::from_rgb(0x53, 0x5A, 0x63);
    let text_muted = Color32::from_rgb(0x87, 0x8E, 0x97);

    let mut v = Visuals::light();
    v.dark_mode = false;
    v.override_text_color = Some(text_primary);
    v.panel_fill = bg_app;
    v.window_fill = surface;
    v.window_stroke = Stroke::new(1.0, border);
    v.window_corner_radius = CornerRadius::same(RADIUS_LG as u8);
    v.window_shadow = shadow_lg(false);
    v.popup_shadow = shadow_sm(false);
    v.extreme_bg_color = surface_2;
    v.faint_bg_color = surface_2;
    v.code_bg_color = surface_2;
    v.hyperlink_color = GREEN_600;
    v.warn_fg_color = ORANGE;
    v.error_fg_color = STATUS_DANGER;

    v.selection = Selection {
        bg_fill: GREEN_500.linear_multiply(0.12),
        stroke: Stroke::new(1.0, GREEN_500),
    };

    v.widgets = Widgets {
        noninteractive: WidgetVisuals {
            bg_fill: bg_app,
            weak_bg_fill: bg_app,
            bg_stroke: Stroke::new(1.0, border),
            fg_stroke: Stroke::new(1.0, text_secondary),
            corner_radius: CornerRadius::same(RADIUS_MD as u8),
            expansion: 0.0,
        },
        inactive: WidgetVisuals {
            bg_fill: surface,
            weak_bg_fill: surface,
            bg_stroke: Stroke::new(1.0, border_strong),
            fg_stroke: Stroke::new(1.0, text_primary),
            corner_radius: CornerRadius::same(RADIUS_MD as u8),
            expansion: 0.0,
        },
        hovered: WidgetVisuals {
            bg_fill: GREEN_600,
            weak_bg_fill: GREEN_500.linear_multiply(0.12),
            bg_stroke: Stroke::new(1.0, GREEN_500),
            fg_stroke: Stroke::new(1.5, WHITE),
            corner_radius: CornerRadius::same(RADIUS_MD as u8),
            expansion: 1.0,
        },
        active: WidgetVisuals {
            bg_fill: GREEN_500,
            weak_bg_fill: GREEN_500,
            bg_stroke: Stroke::new(1.0, GREEN_700),
            fg_stroke: Stroke::new(2.0, WHITE), // text-on-brand
            corner_radius: CornerRadius::same(RADIUS_MD as u8),
            expansion: 0.0,
        },
        open: WidgetVisuals {
            bg_fill: surface,
            weak_bg_fill: surface,
            bg_stroke: Stroke::new(1.0, GREEN_500),
            fg_stroke: Stroke::new(1.0, text_primary),
            corner_radius: CornerRadius::same(RADIUS_MD as u8),
            expansion: 0.0,
        },
    };

    let _ = text_muted;
    let mut style = Style {
        visuals: v,
        ..Default::default()
    };
    install_spacing(&mut style);
    install_text_styles(&mut style);
    style
}
