//! deck: terminals, a browser and the agents that drive them, in one window.
//!
//! One binary is both the app and its control CLI: `deck` opens a window;
//! `deck pane list` (and friends), run from a terminal inside it, drives it.

mod browser;
mod chat;
mod cli;
mod control;
mod favicon;
mod field;
mod fuzzy;
mod history;
mod icon;
mod inspect;
mod inspector;
mod launcher;
mod markdown;
mod menu;
mod net;
mod notify;
mod palette;
mod pasteboard;
mod review;
mod session;
mod settings;
mod settings_pane;
mod tasks;
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
    adopt_shell_path();

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

/// Apps started from Finder or the Dock get launchd's bare `PATH`
/// (`/usr/bin:/bin:…`), not the one your shell builds, so agent CLIs in
/// Homebrew or `~/.local/bin` would look uninstalled. Borrow the login
/// shell's `PATH` before anything else runs (and before any thread exists,
/// which is what makes setting it sound). From a terminal it is already
/// right, so this only runs without one. A slow shell is given 3 seconds.
pub(crate) fn adopt_shell_path() {
    use std::io::Read;
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

    if std::env::var_os("TERM").is_some() {
        return;
    }
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".into());
    let Ok(mut child) = Command::new(&shell)
        .args(["-ilc", "printf '\\n__DECK_PATH__%s\\n' \"$PATH\""])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    else {
        return;
    };
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(20)),
            _ => {
                let _ = child.kill();
                return;
            }
        }
    }
    let mut out = String::new();
    if let Some(mut stdout) = child.stdout.take() {
        let _ = stdout.read_to_string(&mut out);
    }
    if let Some(path) = shell_path(&out) {
        std::env::set_var("PATH", path);
    }
}

/// The `PATH` line from the shell's output, skipping whatever its startup
/// files printed first.
fn shell_path(out: &str) -> Option<&str> {
    out.lines()
        .find_map(|l| l.strip_prefix("__DECK_PATH__"))
        .filter(|p| p.contains('/'))
}

#[cfg(test)]
mod tests {
    use super::shell_path;

    #[test]
    fn the_path_line_is_found_among_shell_noise() {
        let out = "Last login: today\nwelcome!\n__DECK_PATH__/opt/homebrew/bin:/usr/bin\n";
        assert_eq!(shell_path(out), Some("/opt/homebrew/bin:/usr/bin"));
        assert_eq!(shell_path("no marker here"), None);
        assert_eq!(shell_path("__DECK_PATH__\n"), None);
    }
}
