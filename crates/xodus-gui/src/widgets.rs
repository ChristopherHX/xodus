//! Small reusable painters/widgets used by the library grid.

use eframe::icon_data::from_png_bytes;
use egui::{Align2, Color32, CornerRadius, FontFamily, FontId, Mesh, Painter, Pos2, Rect, Response, Sense, Ui, Vec2, pos2};

use crate::library::{Game, GameStatus};
use crate::theme;

pub const CARD_SIZE: Vec2 = Vec2::new(176.0, 176.0);

/// Stable identity for a game's card, independent of its position in the
/// (filterable, reflow-able) grid — lets external code (arrow-key focus
/// navigation) target a specific card without duplicating layout order.
pub fn game_card_id(name: &str) -> egui::Id {
    egui::Id::new(("game_card", name))
}

/// Paints `rect` (rounded by `radius`) filled with a diagonal gradient running
/// from `top_left` to `bottom_right`.
fn gradient_rounded_rect(painter: &Painter, rect: Rect, radius: f32, top_left: Color32, bottom_right: Color32) {
    use std::f32::consts::PI;

    let radius = radius.min(rect.width() / 2.0).min(rect.height() / 2.0).max(0.0);
    const SEGMENTS: usize = 8;

    let corners = [
        (Pos2::new(rect.left() + radius, rect.top() + radius), PI, 1.5 * PI),
        (Pos2::new(rect.right() - radius, rect.top() + radius), 1.5 * PI, 2.0 * PI),
        (Pos2::new(rect.right() - radius, rect.bottom() - radius), 0.0, 0.5 * PI),
        (Pos2::new(rect.left() + radius, rect.bottom() - radius), 0.5 * PI, PI),
    ];

    let diag = rect.width() + rect.height();
    let color_at = |p: Pos2| {
        let t = if diag > 0.0 {
            ((p.x - rect.left()) + (p.y - rect.top())) / diag
        } else {
            0.0
        };
        lerp_color(top_left, bottom_right, t.clamp(0.0, 1.0))
    };

    let mut points = Vec::with_capacity((SEGMENTS + 1) * corners.len());
    for (center, start_angle, end_angle) in corners {
        for i in 0..=SEGMENTS {
            let t = i as f32 / SEGMENTS as f32;
            let angle = start_angle + (end_angle - start_angle) * t;
            points.push(center + Vec2::angled(angle) * radius);
        }
    }

    let mut mesh = Mesh::default();
    mesh.colored_vertex(rect.center(), color_at(rect.center()));
    for p in &points {
        mesh.colored_vertex(*p, color_at(*p));
    }
    let n = points.len() as u32;
    for i in 0..n {
        mesh.add_triangle(0, 1 + i, 1 + (i + 1) % n);
    }
    painter.add(mesh);
}

fn lerp_color(a: Color32, b: Color32, t: f32) -> Color32 {
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgb(l(a.r(), b.r()), l(a.g(), b.g()), l(a.b(), b.b()))
}

/// Paints a rounded pill of text anchored at `pos` and returns its rect.
fn badge(painter: &Painter, pos: Pos2, align: Align2, text: &str, font_id: FontId, bg: Color32, fg: Color32) -> Rect {
    let galley = painter.layout_no_wrap(text.to_owned(), font_id, fg);
    let pad = Vec2::new(9.0, 5.0);
    let rect = align.anchor_size(pos, galley.size() + pad * 2.0);
    painter.rect_filled(rect, CornerRadius::same(u8::MAX), bg);
    painter.galley(rect.min + pad, galley, fg);
    rect
}

/// Paints the (possibly wrapping) game title anchored to the bottom-left of `rect`.
fn title(painter: &Painter, rect: Rect, text: &str, font_id: FontId, color: Color32, pad: f32) {
    let galley = painter.layout(text.to_owned(), font_id, color, rect.width() - pad * 2.0);
    let pos = Pos2::new(rect.left() + pad, rect.bottom() - pad - galley.size().y);
    painter.galley(pos, galley, color);
}

/// Renders one library tile: gradient art, badges, title, and the status line below it.
pub fn game_card(ui: &mut Ui, game: &Game) -> Response {
    ui.vertical(|ui| {
        let (_, rect) = ui.allocate_space(CARD_SIZE);
        let response = ui.interact(rect, game_card_id(game.name), Sense::click());

        // // Keep keyboard/gamepad navigation visible: if this card just became the
        // // focused one (possibly while scrolled out of view), scroll it into frame.
        if response.gained_focus() {
            response.scroll_to_me(Some(egui::Align::Center));
        }

        if ui.is_rect_visible(rect) {
            let painter = ui.painter();
            gradient_rounded_rect(painter, rect, theme::RADIUS_XL, game.gradient.0, game.gradient.1);

            if let Some(tex) = &game.cover_image {
                painter.image(tex.id(), rect.shrink(32.0), Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
                //gradient_rounded_rect(painter, rect, theme::RADIUS_XL, Color32::from_rgba_unmultiplied(game.gradient.0.r(), game.gradient.0.g(), game.gradient.0.b(), 200), Color32::from_rgba_unmultiplied(game.gradient.1.r(), game.gradient.1.g(), game.gradient.1.b(), 200));
            }

            let badge_font = FontId::new(11.5, FontFamily::Proportional);
            if game.game_pass {
                badge(
                    painter,
                    rect.left_top() + Vec2::new(10.0, 10.0),
                    Align2::LEFT_TOP,
                    "Game Pass",
                    badge_font.clone(),
                    theme::GREEN_500,
                    Color32::WHITE,
                );
            }
            if game.cloud {
                badge(
                    painter,
                    rect.right_top() + Vec2::new(-10.0, 10.0),
                    Align2::RIGHT_TOP,
                    "Cloud",
                    badge_font,
                    Color32::from_rgba_unmultiplied(255, 255, 255, 235),
                    Color32::from_rgb(0x2A, 0x2E, 0x33),
                );
            }

            title(
                painter,
                rect,
                game.name,
                FontId::new(17.0, FontFamily::Name("Display".into())),
                Color32::WHITE,
                12.0,
            );

            if response.hovered() {
                painter.rect_stroke(
                    rect.shrink(1.0),
                    CornerRadius::same(theme::RADIUS_XL as u8),
                    egui::Stroke::new(2.0, Color32::from_rgba_unmultiplied(255, 255, 255, 140)),
                    egui::StrokeKind::Inside,
                );
            }

            // Keyboard/gamepad focus ring: brighter and thicker than the hover ring so it
            // stays visible even when the pointer is elsewhere.
            if response.has_focus() {
                painter.rect_stroke(
                    rect.shrink(1.5),
                    CornerRadius::same(theme::RADIUS_XL as u8),
                    egui::Stroke::new(3.0, theme::GREEN_400),
                    egui::StrokeKind::Inside,
                );
            }
        }

        ui.add_space(8.0);
        status_line(ui, game.status);

        response
    })
    .inner
}

fn status_line(ui: &mut Ui, status: GameStatus) {
    match status {
        GameStatus::Installed => pill_label(ui, "Installed", theme::GREEN_500.linear_multiply(0.16), theme::GREEN_700),
        GameStatus::Installing(pct) => pill_label(
            ui,
            &format!("Installing {pct} %"),
            theme::ORANGE.linear_multiply(0.20),
            Color32::from_rgb(0x9A, 0x5A, 0x00),
        ),
        GameStatus::Size(size) => {
            ui.label(egui::RichText::new(size).size(13.0).color(ui.visuals().weak_text_color()));
        }
    }
}

fn pill_label(ui: &mut Ui, text: &str, bg: Color32, fg: Color32) {
    egui::Frame::new()
        .fill(bg)
        .corner_radius(CornerRadius::same(u8::MAX))
        .inner_margin(egui::Margin::symmetric(10, 4))
        .show(ui, |ui| {
            ui.label(egui::RichText::new(text).size(12.5).color(fg).strong());
        });
}
