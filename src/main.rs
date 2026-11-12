//! StynPS PlaySelect: application entry point.

mod daemon;
mod remap;
mod system;
mod ui;

use std::thread;

use eframe::egui;

fn main() -> eframe::Result<()> {
    tracing_subscriber::fmt::init();

    // Backend daemon on a background thread
    let spawned = thread::Builder::new().name("stynps-daemon".into()).spawn(|| {
        let rt = match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
            Ok(rt) => rt,
                                                                            Err(e) => {
                                                                                eprintln!("[stynps] failed to start async runtime for the daemon: {e}");
                                                                                return;
                                                                            }
        };
        if let Err(e) = rt.block_on(daemon::run()) {
            eprintln!("[stynps] daemon stopped: {e:#}");
        }
    });
    if let Err(e) = spawned {
        eprintln!("[stynps] could not spawn daemon thread: {e}");
    }

    // UI on the main thread
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
        .with_title("StynPS PlaySelect")
        .with_app_id("stynps")
        .with_inner_size([1180.0, 760.0])
        .with_min_inner_size([960.0, 640.0]),
        ..Default::default()
    };
    let result = eframe::run_native(
        "StynPS PlaySelect",
        options,
        Box::new(|cc| {
            egui_extras::install_image_loaders(&cc.egui_ctx);
            Ok(Box::new(ui::App::new(cc)))
        }),
    );

    daemon::cleanup_socket();
    result
}
