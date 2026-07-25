use egui::{Align, Align2, Color32, CornerRadius, FontFamily, FontId, Layout, Margin, Pos2, RichText, Sense, Vec2};

use crate::library::{self, Game, GameStatus};
use crate::theme;
use crate::widgets;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Nav {
    Home,
    Library,
    Store,
    Downloads,
    Installed,
    Recent,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Filter {
    All,
    Installed,
    GamePass,
    Recent,
    R,
}

/// Which single element in the nav list / filter row / game grid currently
/// has keyboard (or gamepad) focus, in an app-owned model we drive ourselves
/// -- see the comment on `next_focus_target` for why.
#[derive(Clone, Copy, PartialEq, Eq)]
enum FocusTarget {
    Nav(Nav),
    Filter(Filter),
    Card(&'static str),
}

#[derive(Clone, Copy)]
enum Dir {
    Up,
    Down,
    Left,
    Right,
}

const NAV_ORDER: [Nav; 6] = [
    Nav::Home,
    Nav::Library,
    Nav::Store,
    Nav::Downloads,
    Nav::Installed,
    Nav::Recent,
];

// Visual left-to-right order of the filter pills (the row is built
// right-to-left in `library_view`, so this is the reverse of insertion order).
const FILTER_ORDER: [Filter; 4] = [Filter::All, Filter::Installed, Filter::GamePass, Filter::Recent];

/// The stable `Id` a given focus target's widget is interacted with under.
/// Computed independently of layout/call order (see `widgets::game_card_id`),
/// so we can point egui's focus at a target without having drawn it yet.
fn focus_id(target: FocusTarget) -> egui::Id {
    match target {
        FocusTarget::Nav(nav) => egui::Id::new(("nav_item", nav as u8)),
        FocusTarget::Filter(filter) => egui::Id::new(("filter_pill", filter as u8)),
        FocusTarget::Card(name) => widgets::game_card_id(name),
    }
}

/// Decide where arrow-key focus goes next, region by region.
///
/// egui ships a generic "nearest focusable widget in this direction" search
/// that runs automatically on arrow keys. It has no idea our game grid is a
/// grid, though -- it just measures raw screen distance against every
/// focusable widget on screen, sidebar/top-bar included. In a dense wrapping
/// grid that search can end up picking the search box or the gamepad-mode
/// button over the adjacent card, especially near grid edges. So instead we
/// track focus ourselves and move it with row/column math scoped to whichever
/// region (nav list, filter row, or grid) it's currently in; egui's own
/// search is switched off for the frame (see `XodusApp::handle_focus_navigation`).
fn next_focus_target(current: FocusTarget, dir: Dir, columns: usize, games: &[&Game]) -> FocusTarget {
    let columns = columns.max(1);
    match current {
        FocusTarget::Nav(nav) => {
            let idx = NAV_ORDER.iter().position(|n| *n == nav).unwrap_or(0);
            match dir {
                Dir::Up => FocusTarget::Nav(NAV_ORDER[idx.saturating_sub(1)]),
                Dir::Down => FocusTarget::Nav(NAV_ORDER[(idx + 1).min(NAV_ORDER.len() - 1)]),
                Dir::Right => FocusTarget::Filter(FILTER_ORDER[0]),
                Dir::Left => current,
            }
        }
        FocusTarget::Filter(filter) => {
            let idx = FILTER_ORDER.iter().position(|f| *f == filter).unwrap_or(0);
            match dir {
                Dir::Left => {
                    if idx == 0 {
                        FocusTarget::Nav(Nav::Library)
                    } else {
                        FocusTarget::Filter(FILTER_ORDER[idx - 1])
                    }
                }
                Dir::Right => FocusTarget::Filter(FILTER_ORDER[(idx + 1).min(FILTER_ORDER.len() - 1)]),
                Dir::Down => games.first().map(|g| FocusTarget::Card(g.name)).unwrap_or(current),
                Dir::Up => current,
            }
        }
        FocusTarget::Card(name) => {
            let Some(idx) = games.iter().position(|g| g.name == name) else {
                return current;
            };
            let row = idx / columns;
            let col = idx % columns;
            match dir {
                Dir::Left => {
                    if col == 0 {
                        current
                    } else {
                        FocusTarget::Card(games[idx - 1].name)
                    }
                }
                Dir::Right => {
                    if col + 1 >= columns || idx + 1 >= games.len() {
                        current
                    } else {
                        FocusTarget::Card(games[idx + 1].name)
                    }
                }
                Dir::Up => {
                    if row == 0 {
                        FocusTarget::Filter(FILTER_ORDER[col.min(FILTER_ORDER.len() - 1)])
                    } else {
                        FocusTarget::Card(games[idx - columns].name)
                    }
                }
                Dir::Down => {
                    let target_idx = idx + columns;
                    if target_idx >= games.len() {
                        current
                    } else {
                        FocusTarget::Card(games[target_idx].name)
                    }
                }
            }
        }
    }
}

pub struct XodusApp {
    search: String,
    nav: Nav,
    filter: Filter,
    games: Vec<Game>,
    focused: FocusTarget,
    /// Grid column count from the previous frame's layout pass -- one frame
    /// stale at worst, which is invisible in practice (window resizes are
    /// rare/gradual; keypresses aren't), and avoids re-deriving layout math
    /// here before the grid itself has run for this frame.
    grid_columns: usize,
}

impl XodusApp {
    pub fn new(cc: &eframe::CreationContext) -> Self {
        egui_extras::install_image_loaders(&cc.egui_ctx);
        theme::install_fonts(&cc.egui_ctx);
        cc.egui_ctx
            .set_style_of(egui::Theme::Dark, theme::xodus_dark());
        cc.egui_ctx
            .set_style_of(egui::Theme::Light, theme::xodus_light());

        Self {
            search: String::new(),
            nav: Nav::Library,
            filter: Filter::All,
            games: library::library(&cc.egui_ctx),
            focused: FocusTarget::Nav(Nav::Library),
            grid_columns: 1,
        }
    }

    /// Run once per frame, before any widget is drawn. Bootstraps initial
    /// focus, and -- when an arrow key was pressed while focus is somewhere
    /// in our nav/filter/grid model -- computes the next target ourselves and
    /// cancels egui's built-in directional search so it can't override us.
    ///
    /// If focus is currently on something outside that model (e.g. the search
    /// box, reached via Tab), we leave arrow keys alone entirely, so they
    /// still move a text cursor instead of hijacking navigation.
    fn handle_focus_navigation(&mut self, ctx: &egui::Context) {
        let current = ctx.memory(|m| m.focused());

        if current.is_none() {
            ctx.memory_mut(|m| m.request_focus(focus_id(self.focused)));
            return;
        }

        if current != Some(focus_id(self.focused)) {
            return;
        }

        let dir = ctx.input(|i| {
            if i.key_pressed(egui::Key::ArrowUp) {
                Some(Dir::Up)
            } else if i.key_pressed(egui::Key::ArrowDown) {
                Some(Dir::Down)
            } else if i.key_pressed(egui::Key::ArrowLeft) {
                Some(Dir::Left)
            } else if i.key_pressed(egui::Key::ArrowRight) {
                Some(Dir::Right)
            } else {
                None
            }
        });
        let Some(dir) = dir else { return };

        ctx.memory_mut(|m| m.move_focus(egui::FocusDirection::None));

        let games: Vec<&Game> = self.games.iter().filter(|g| Self::filter_matches(self.filter, g)).collect();
        self.focused = next_focus_target(self.focused, dir, self.grid_columns, &games);
        ctx.memory_mut(|m| m.request_focus(focus_id(self.focused)));
    }

    fn top_bar(&mut self, ui: &mut egui::Ui) {
        egui::Panel::top("top_bar")
            .exact_size(64.0)
            .frame(
                egui::Frame::NONE
                    .fill(ui.visuals().panel_fill)
                    .inner_margin(Margin::symmetric(24, 0)),
            )
            .show(ui, |ui| {
                ui.columns(3, |cols| {
                    cols[1].with_layout(Layout::top_down(Align::Center), |ui| {
                        ui.add_space(14.0);
                        self.search_box(ui);
                    });

                    cols[2].with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.add(
                            egui::Button::new("Gamepad mode")
                                .corner_radius(CornerRadius::same(u8::MAX))
                                .min_size(Vec2::new(0.0, 36.0)),
                        );
                    });
                });
            });
    }

    fn search_box(&mut self, ui: &mut egui::Ui) {
        let kbd_font = FontId::new(11.0, FontFamily::Monospace);
        let kbd_galley = ui.painter().layout_no_wrap(
            "\u{2318}K".to_owned(),
            kbd_font,
            ui.visuals().weak_text_color(),
        );
        let kbd_pad = Vec2::new(7.0, 4.0);
        let kbd_size = kbd_galley.size() + kbd_pad * 2.0;

        egui::Frame::NONE
            .fill(ui.visuals().extreme_bg_color)
            .corner_radius(CornerRadius::same(10))
            .inner_margin(Margin::symmetric(14, 8))
            .show(ui, |ui| {
                ui.set_width(420.0);
                ui.horizontal(|ui| {
                    let remaining = (ui.available_width() - kbd_size.x - 8.0).max(0.0);
                    ui.add(
                        egui::TextEdit::singleline(&mut self.search)
                            .frame(egui::Frame::NONE)
                            .hint_text("Search games, store & settings")
                            .desired_width(remaining),
                    );
                    let (rect, _) = ui.allocate_exact_size(kbd_size, Sense::hover());
                    ui.painter()
                        .rect_filled(rect, CornerRadius::same(6), ui.visuals().window_fill);
                    ui.painter()
                        .galley(rect.min + kbd_pad, kbd_galley, ui.visuals().weak_text_color());
                });
            });
    }

    fn sidebar(&mut self, ui: &mut egui::Ui) {
        egui::Panel::left("sidebar")
            .exact_size(240.0)
            .resizable(false)
            .frame(
                egui::Frame::NONE
                    .fill(ui.visuals().panel_fill)
                    .inner_margin(Margin::symmetric(16, 20)),
            )
            .show(ui, |ui| {
                section_label(ui, "PLAY");
                ui.add_space(4.0);
                self.nav_item(ui, Nav::Home, "Home");
                self.nav_item(ui, Nav::Library, "Library");
                self.nav_item(ui, Nav::Store, "Store");
                self.nav_item(ui, Nav::Downloads, "Downloads");

                ui.add_space(18.0);
                section_label(ui, "COLLECTIONS");
                ui.add_space(4.0);
                self.nav_item(ui, Nav::Installed, "Installed");
                self.nav_item(ui, Nav::Recent, "Recently played");

                let reserved_bottom = 118.0;
                ui.add_space((ui.available_height() - reserved_bottom).max(0.0));
                ui.label(RichText::new("Settings").size(14.5));
                ui.add_space(10.0);
                user_card(ui, "TX", "tux_rider", true);
            });
    }

    fn nav_item(&mut self, ui: &mut egui::Ui, item: Nav, label: &str) {
        let selected = self.nav == item;
        let desired = Vec2::new(ui.available_width(), 36.0);
        let (_, rect) = ui.allocate_space(desired);
        let response = ui.interact(rect, focus_id(FocusTarget::Nav(item)), Sense::click());
        if response.clicked() {
            self.nav = item;
            self.focused = FocusTarget::Nav(item);
        }

        if ui.is_rect_visible(rect) {
            if selected {
                ui.painter().rect_filled(
                    rect,
                    CornerRadius::same(10),
                    theme::GREEN_500.linear_multiply(0.16),
                );
            } else if response.hovered() {
                ui.painter().rect_filled(
                    rect,
                    CornerRadius::same(10),
                    ui.visuals().widgets.hovered.weak_bg_fill,
                );
            }
            let color = if selected {
                theme::GREEN_700
            } else {
                ui.visuals().text_color()
            };
            ui.painter().text(
                Pos2::new(rect.left() + 14.0, rect.center().y),
                Align2::LEFT_CENTER,
                label,
                FontId::new(15.0, FontFamily::Proportional),
                color,
            );

            if response.has_focus() {
                ui.painter().rect_stroke(
                    rect.shrink(1.0),
                    CornerRadius::same(10),
                    egui::Stroke::new(2.0, theme::GREEN_400),
                    egui::StrokeKind::Inside,
                );
            }
        }
    }

    fn library_view(&mut self, ui: &mut egui::Ui) {
        ui.add_space(28.0);
        ui.horizontal(|ui| {
            ui.add_space(28.0);
            ui.vertical(|ui| {
                ui.heading("Your library");
                ui.add_space(4.0);
                ui.label(
                    RichText::new(format!(
                        "{} games \u{b7} running on Linux via Proton-XO 9.0",
                        self.games.len()
                    ))
                    .color(ui.visuals().weak_text_color()),
                );
            });

            // ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            //     ui.add_space(28.0);
            //     self.filter_pill(ui, Filter::Recent, "Recent");
            //     self.filter_pill(ui, Filter::GamePass, "Game Pass");
            //     self.filter_pill(ui, Filter::Installed, "Installed");
            //     self.filter_pill(ui, Filter::All, "All");
            // });
            // ui.take_available_space();

            ui.with_layout(Layout::left_to_right(Align::Center).with_main_align(Align::Max), |ui| {
                // ui.with_layout(Layout::left_to_right(Align::Center).with_main_align(Align::Max), |ui| {
                // });
                let available_width = ui.available_width();

                // ui.horizontal(|ui| {
                //     self.filter_pill(ui, Filter::All, "All");
                // });
                self.filter_pill(ui, Filter::All, "All");
                self.filter_pill(ui, Filter::Installed, "Installed");
                self.filter_pill(ui, Filter::GamePass, "Game Pass");
                self.filter_pill(ui, Filter::Recent, "Recent");
                let available_width2 = ui.available_width();
                self.filter_pill(ui, Filter::R, &format!("{} {}", available_width, available_width2));
            });

            // ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            //     // ui.add_space(28.0);
            //     ui.horizontal(|ui| {
            //         self.filter_pill(ui, Filter::All, "All");
            //         self.filter_pill(ui, Filter::Installed, "Installed");
            //         self.filter_pill(ui, Filter::GamePass, "Game Pass");
            //         self.filter_pill(ui, Filter::Recent, "Recent");
            //     });
            // });
        });

        ui.add_space(20.0);

        let games: Vec<&Game> = self
            .games
            .iter()
            .filter(|game| Self::filter_matches(self.filter, game))
            .collect();

        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.add_space(28.0);
                    ui.vertical(|ui| {
                        let gutter = ui.spacing().item_spacing.x;
                        let col_width = widgets::CARD_SIZE.x + gutter;
                        let columns = ((ui.available_width() + gutter) / col_width).floor().max(1.0) as usize;
                        self.grid_columns = columns;

                        for row in games.chunks(columns) {
                            ui.horizontal(|ui| {
                                for game in row {
                                    let response = widgets::game_card(ui, game);
                                    if response.clicked() {
                                        self.focused = FocusTarget::Card(game.name);
                                    }
                                }
                            });
                            ui.add_space(8.0);
                        }
                    });
                });
                ui.add_space(20.0);
            });
    }

    fn filter_pill(&mut self, ui: &mut egui::Ui, filter: Filter, label: &str) {
        let selected = self.filter == filter;
        let font = FontId::new(14.0, FontFamily::Proportional);
        let text_size = ui
            .painter()
            .layout_no_wrap(label.to_owned(), font.clone(), Color32::WHITE)
            .size();
        let pad = Vec2::new(16.0, 9.0);
        let (_, rect) = ui.allocate_space(text_size + pad * 2.0);
        let response = ui.interact(rect, focus_id(FocusTarget::Filter(filter)), Sense::click());
        if response.clicked() {
            self.filter = filter;
            self.focused = FocusTarget::Filter(filter);
        }

        if ui.is_rect_visible(rect) {
            let (bg, fg) = if selected {
                (theme::GREEN_500, Color32::WHITE)
            } else if response.hovered() {
                (ui.visuals().widgets.hovered.weak_bg_fill, ui.visuals().text_color())
            } else {
                (ui.visuals().widgets.inactive.weak_bg_fill, ui.visuals().text_color())
            };
            ui.painter().rect_filled(rect, CornerRadius::same(u8::MAX), bg);
            ui.painter().text(rect.center(), Align2::CENTER_CENTER, label, font, fg);

            if response.has_focus() {
                ui.painter().rect_stroke(
                    rect.shrink(1.0),
                    CornerRadius::same(u8::MAX),
                    egui::Stroke::new(2.0, theme::GREEN_400),
                    egui::StrokeKind::Inside,
                );
            }
        }

        ui.add_space(8.0);
    }

    fn filter_matches(filter: Filter, game: &Game) -> bool {
        match filter {
            Filter::All => true,
            Filter::Installed => matches!(game.status, GameStatus::Installed),
            Filter::GamePass => game.game_pass,
            Filter::Recent => true,
            Filter::R => true,
        }
    }
}

fn section_label(ui: &mut egui::Ui, text: &str) {
    ui.label(
        RichText::new(text)
            .size(11.0)
            .extra_letter_spacing(1.2)
            .color(ui.visuals().weak_text_color())
            .strong(),
    );
}

fn user_card(ui: &mut egui::Ui, initials: &str, name: &str, online: bool) {
    egui::Frame::NONE
        .fill(ui.visuals().extreme_bg_color)
        .corner_radius(CornerRadius::same(14))
        .inner_margin(Margin::symmetric(10, 10))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                let avatar_size = 36.0;
                let (rect, _) = ui.allocate_exact_size(Vec2::splat(avatar_size), Sense::hover());
                ui.painter()
                    .circle_filled(rect.center(), avatar_size / 2.0, theme::GREEN_500);
                ui.painter().text(
                    rect.center(),
                    Align2::CENTER_CENTER,
                    initials,
                    FontId::new(13.0, FontFamily::Proportional),
                    Color32::WHITE,
                );

                ui.vertical(|ui| {
                    ui.label(RichText::new(name).size(14.0).color(ui.visuals().text_color()));
                    ui.horizontal(|ui| {
                        let dot_color = if online {
                            theme::GREEN_500
                        } else {
                            ui.visuals().weak_text_color()
                        };
                        let (dot_rect, _) = ui.allocate_exact_size(Vec2::splat(8.0), Sense::hover());
                        ui.painter().circle_filled(dot_rect.center(), 3.0, dot_color);
                        ui.label(
                            RichText::new(if online { "Online" } else { "Offline" })
                                .size(12.0)
                                .color(ui.visuals().weak_text_color()),
                        );
                    });
                });
            });
        });
}

impl eframe::App for XodusApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.handle_focus_navigation(ui.ctx());
        self.top_bar(ui);
        self.sidebar(ui);
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(ui.visuals().window_fill))
            .show(ui, |ui| {
                self.library_view(ui);
            });
    }
}
