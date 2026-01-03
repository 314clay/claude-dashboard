//! Native Claude Activity Dashboard
//!
//! A high-performance desktop app for visualizing Claude Code sessions.

mod api;
mod app;
mod graph;

use eframe::egui;
use tracing_subscriber;

fn main() -> eframe::Result<()> {
    // Initialize logging
    tracing_subscriber::fmt::init();

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1400.0, 900.0])
            .with_title("Claude Activity Dashboard"),
        persist_window: true, // Persist window state and egui memory between sessions
        ..Default::default()
    };

    eframe::run_native(
        "Claude Activity Dashboard",
        options,
        Box::new(|cc| Ok(Box::new(app::DashboardApp::new(cc)))),
    )
}
