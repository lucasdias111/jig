//! Jig: a code editor where AI works through small commands at the cursor.

mod document;
mod file_tree;
mod menus;
mod project;
mod theme;
mod workspace;

use std::path::PathBuf;

use gpui_kit::component::TitleBar;
use gpui_kit::*;
use workspace::{Quit, Workspace};

fn main() {
    let mut paths = std::env::args_os().skip(1).map(PathBuf::from);
    let path = paths.next();
    // Any further files open in tabs of their own.
    let more: Vec<PathBuf> = paths.collect();

    gpui_kit::application().run(move |cx| {
        gpui_kit::init(cx);
        theme::init(cx);
        cx.bind_keys(workspace::key_bindings());
        menus::init(cx);
        // Reached only when no window handles Quit (e.g. none is open).
        cx.on_action(|_: &Quit, cx| cx.quit());
        cx.on_window_closed(|cx, _| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();

        // A transparent titlebar: the workspace draws its own, so only the
        // traffic lights and the code remain. On macOS the window is
        // translucent so the sidebar picks up the desktop's vibrancy; the
        // editor paints its own opaque background.
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::centered(size(px(1080.), px(760.)), cx)),
            window_min_size: Some(size(px(480.), px(320.))),
            titlebar: Some(TitlebarOptions {
                // Centred in the workspace's taller title bar.
                traffic_light_position: Some(point(px(16.), px(16.))),
                ..TitleBar::title_bar_options()
            }),
            window_background: if cfg!(target_os = "macos") {
                WindowBackgroundAppearance::Blurred
            } else {
                WindowBackgroundAppearance::Opaque
            },
            ..TitleBar::window_options()
        };
        gpui_kit::open_window(options, cx, |window, cx| {
            cx.new(|cx| {
                let mut workspace = Workspace::new(path, window, cx);
                for path in &more {
                    workspace.open_file(path, window, cx);
                }
                workspace
            })
        })
        .expect("failed to open window");
        cx.activate(true);
    });
}
