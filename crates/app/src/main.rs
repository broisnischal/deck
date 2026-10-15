//! deck: terminals, a browser and the agents that drive them, in one window.
//!
//! One binary is both the app and its control CLI: `deck` opens a window;
//! `deck pane list` (and friends), run from a terminal inside it, drives it.

mod browser;
mod chat;
mod cli;
mod control;
mod field;
mod icon;
mod inspect;
mod inspector;
mod markdown;
mod pasteboard;
mod settings;
mod terminal;
mod textbuf;
mod theme;
mod threads;
mod workspace;

use gpui::prelude::*;
use gpui::{
    point, px, size, App, Application, Bounds, TitlebarOptions, WindowBounds, WindowOptions,
};
use std::path::PathBuf;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if !args.is_empty() {
        std::process::exit(cli::run(&args));
    }

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    // Launched from Finder the cwd is `/`; home is the useful default there.
    let cwd = std::env::current_dir()
        .ok()
        .filter(|d| d.as_os_str() != "/")
        .or_else(|| std::env::var_os("HOME").map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("/"));

    Application::new()
        .with_assets(icon::Assets)
        .run(move |cx: &mut App| {
            workspace::install(cx);
            cx.on_window_closed(|cx| {
                if cx.windows().is_empty() {
                    cx.quit();
                }
            })
            .detach();

            let bounds = Bounds::centered(None, size(px(1360.), px(860.)), cx);
            let background = std::env::var_os("DECK_BACKGROUND").is_some();
            cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    // The sidebar draws under a transparent titlebar, with the
                    // traffic lights sitting in its top strip.
                    titlebar: Some(TitlebarOptions {
                        title: Some("Deck".into()),
                        appears_transparent: true,
                        traffic_light_position: Some(point(px(14.), px(14.))),
                    }),
                    // Scripted checks open a window that does not take the
                    // keyboard from whatever the user is typing in.
                    focus: !background,
                    ..Default::default()
                },
                |window, cx| cx.new(|cx| workspace::Workspace::new(cwd.clone(), window, cx)),
            )
            .expect("failed to open window");
            if !background {
                cx.activate(true);
            }
        });
}
