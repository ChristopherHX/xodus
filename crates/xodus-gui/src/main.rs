mod app;
mod library;
mod theme;
mod widgets;

fn main() {
    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1180.0, 760.0])
            .with_min_inner_size([860.0, 560.0])
            .with_icon(
                // NOTE: Adding an icon is optional
                eframe::icon_data::from_png_bytes(
                    &include_bytes!("../../../assets/Icon/Icon512.png")[..],
                )
                .expect("Failed to load icon"),
            )
            .with_title("Xodus"),
        ..Default::default()
    };

    eframe::run_native(
        "io.github.xodus-gaming.xodus",
        native_options,
        Box::new(|cc| Ok(Box::new(app::XodusApp::new(cc)))),
    )
    .expect("Faield to run native app");
}
