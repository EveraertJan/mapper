//! RUSTMAP application entry point.

// Release builds on Windows open no console window next to the app.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod app;
mod control;
mod document;
mod guides;
mod led;
mod media;
mod monitors;
mod output_window;

use tracing_subscriber::EnvFilter;

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("info,wgpu_core=warn,wgpu_hal=warn,naga=warn")),
        )
        .init();

    let options = app::Options {
        profiler: std::env::args().any(|a| a == "--profile"),
        images: std::env::args_os()
            .skip(1)
            .filter(|a| !a.to_string_lossy().starts_with("--"))
            .map(std::path::PathBuf::from)
            .collect(),
    };

    let event_loop = winit::event_loop::EventLoop::new()?;
    let mut app = app::App::new(options);
    event_loop.run_app(&mut app)?;
    app.into_result()
}
