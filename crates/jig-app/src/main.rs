//! Jig: a code editor where AI works through small commands at the cursor.

mod document;
mod menus;
mod theme;
mod workspace;

use std::path::PathBuf;

use gpui_kit::component::TitleBar;
use gpui_kit::*;
use workspace::{Quit, Workspace};

fn main() {
    let path = std::env::args_os().nth(1).map(PathBuf::from);

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
        // traffic lights and the code remain.
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::centered(size(px(960.), px(720.)), cx)),
            window_min_size: Some(size(px(480.), px(320.))),
            ..TitleBar::window_options()
        };
        gpui_kit::open_window(options, cx, |window, cx| {
            cx.new(|cx| Workspace::new(path, window, cx))
        })
        .expect("failed to open window");
        cx.activate(true);
    });
}
