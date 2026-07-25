//! Static sample data for the library grid.

use std::default;

use egui::{Color32, epaint};

#[derive(Clone, Copy, PartialEq, Default)]
pub enum GameStatus {
    #[default]
    Installed,
    Installing(u8),
    Size(&'static str),
}

#[derive(Clone, Default)]
pub struct Game {
    pub name: &'static str,
    pub gradient: (Color32, Color32),
    pub game_pass: bool,
    pub cloud: bool,
    pub status: GameStatus,
    pub cover_image: Option<epaint::TextureHandle>,
}

const fn rgb(r: u8, g: u8, b: u8) -> Color32 {
    Color32::from_rgb(r, g, b)
}

pub fn library(ctx: &egui::Context) -> Vec<Game> {
    let icon = eframe::icon_data::from_png_bytes(
        &include_bytes!("../../../assets/Icon/Icon512.png")[..],
    )
    .expect("Failed to load icon");

    let image = egui::ColorImage::from_rgba_unmultiplied([icon.width as usize, icon.height as usize], &icon.rgba);
    let texture = ctx.load_texture(
        "cover-image",
        image,
        egui::TextureOptions::LINEAR,
    );

    use GameStatus::*;
    vec![
        Game {
            name: "Forza Horizon 5",
            gradient: (rgb(0x8A, 0x5C, 0xF6), rgb(0x2F, 0x6B, 0xFF)),
            game_pass: true,
            cloud: false,
            status: Installed,
            cover_image: Some(texture),
        },
        Game {
            name: "Starfield",
            gradient: (rgb(0x33, 0x36, 0x78), rgb(0x4B, 0x40, 0xBE)),
            game_pass: true,
            cloud: false,
            status: Installed,
            ..Default::default()
        },
        Game {
            name: "Halo Infinite",
            gradient: (rgb(0x0E, 0x5C, 0x1E), rgb(0x2F, 0xB6, 0x2A)),
            game_pass: true,
            cloud: false,
            status: Installing(62),
            ..Default::default()
        },
        Game {
            name: "Sea of Thieves",
            gradient: (rgb(0x0E, 0x5A, 0x6B), rgb(0x1F, 0x8C, 0xC9)),
            game_pass: true,
            cloud: false,
            status: Installed,
            ..Default::default()
        },
        Game {
            name: "Gears 5",
            gradient: (rgb(0x5C, 0x14, 0x18), rgb(0xB4, 0x2A, 0x2E)),
            game_pass: true,
            cloud: false,
            status: Installed,
            ..Default::default()
        },
        Game {
            name: "Hi-Fi Rush",
            gradient: (rgb(0xD1, 0x2E, 0x6B), rgb(0xF3, 0x9A, 0x2A)),
            game_pass: true,
            cloud: false,
            status: Installed,
            ..Default::default()
        },
        Game {
            name: "Flight Simulator",
            gradient: (rgb(0x14, 0x3E, 0x8C), rgb(0x2F, 0x8F, 0xE0)),
            game_pass: true,
            cloud: true,
            status: Size("150 GB"),
            ..Default::default()
        },
        Game {
            name: "Ori and the Will",
            gradient: (rgb(0x0E, 0x5C, 0x5A), rgb(0x1F, 0x8C, 0x7A)),
            game_pass: false,
            cloud: false,
            status: Installed,
            ..Default::default()
        },
        Game {
            name: "Hollow Knight",
            gradient: (rgb(0x0B, 0x24, 0x3B), rgb(0x14, 0x54, 0x63)),
            game_pass: false,
            cloud: false,
            status: Installed,
            ..Default::default()
        },
        Game {
            name: "Hades",
            gradient: (rgb(0x6B, 0x1A, 0x2E), rgb(0x5C, 0x2A, 0x7A)),
            game_pass: false,
            cloud: true,
            status: Size("15 GB"),
            ..Default::default()
        },
        Game {
            name: "Psychonauts 2",
            gradient: (rgb(0x7A, 0x3E, 0x14), rgb(0xC9, 0x7A, 0x2A)),
            game_pass: true,
            cloud: false,
            status: Installed,
            ..Default::default()
        },
        Game {
            name: "Grounded",
            gradient: (rgb(0x3B, 0x5C, 0x1A), rgb(0x6B, 0x8C, 0x2A)),
            game_pass: true,
            cloud: false,
            status: Installed,
            ..Default::default()
        },
        Game {
            name: "Pentiment",
            gradient: (rgb(0x7A, 0x2A, 0x14), rgb(0xC9, 0x5A, 0x2A)),
            game_pass: true,
            cloud: true,
            status: Size("8 GB"),
            ..Default::default()
        },
        Game {
            name: "Age of Empires IV",
            gradient: (rgb(0x1A, 0x4B, 0x2A), rgb(0x3E, 0x7A, 0x3E)),
            game_pass: false,
            cloud: false,
            status: Installed,
            ..Default::default()
        },
        Game {
            name: "Minecraft",
            gradient: (rgb(0x0E, 0x5A, 0x4B), rgb(0x1F, 0x9E, 0x7A)),
            game_pass: true,
            cloud: false,
            status: Installed,
            ..Default::default()
        },
    ]
}
