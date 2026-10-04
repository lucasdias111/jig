//! Jig: a code editor where AI works through small commands at the cursor.

mod agent;
mod branch_picker;
mod completions;
mod dap;
mod debug_target;
mod debuggers;
mod definitions;
mod diff;
mod document;
mod file_icons;
mod file_tree;
mod find_in_files;
mod fuzzy;
mod git;
mod git_panel;
mod indentation;
mod languages;
mod lsp;
mod menus;
mod project;
mod project_search;
mod providers;
mod quick_open;
mod recent;
mod run_configs;
mod run_output;
mod run_picker;
mod settings;
mod settings_window;
mod snippets;
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
        let settings_error = settings::init(cx);
        providers::init(cx);
        recent::init(cx);
        agent::init(cx);
        theme::init(cx);
        theme::apply(None, cx);
        cx.bind_keys(workspace::key_bindings());
        // Before the menus: they show only shortcuts bound by then.
        settings_window::init(cx);
        menus::init(cx);
        // Reached only when no window handles Quit (e.g. none is open).
        cx.on_action(|_: &Quit, cx| cx.quit());
        // Settings alone can't open a file, so it doesn't keep Jig running.
        cx.on_window_closed(|cx, _| {
            let windows = cx.windows();
            if windows
                .iter()
                .all(|window| settings_window::is_settings_window(*window, cx))
            {
                cx.quit();
            }
        })
        .detach();

        let window = open_window(path, &more, cx).expect("failed to open window");
        cx.activate(true);
        if let Some(error) = settings_error {
            window
                .update(cx, |_, window, cx| {
                    let detail = format!("Using the defaults until it's fixed. {error}");
                    // Nothing to do with the answer.
                    drop(window.prompt(
                        PromptLevel::Warning,
                        "Your settings couldn't be read",
                        Some(&detail),
                        &["OK"],
                        cx,
                    ));
                })
                .ok();
        }
    });
}

/// Open a Jig window on `path` (a file, a folder or nothing), with `more`
/// files in tabs of their own.
pub fn open_window(
    path: Option<PathBuf>,
    more: &[PathBuf],
    cx: &mut App,
) -> Result<WindowHandle<gpui_kit::base::Root>> {
    // A transparent titlebar: the workspace draws its own, so only the
    // traffic lights and the code remain. On macOS the window is
    // translucent so the sidebar picks up the desktop's vibrancy, and
    // the editor as much as the translucency setting lets it.
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
    cx.open_window(options, |window, cx| {
        let workspace = cx.new(|cx| {
            let mut workspace = Workspace::new(path, window, cx);
            for path in more {
                workspace.open_file(path, window, cx);
            }
            workspace
        });
        // The root would paint the theme's background over the
        // whole window, hiding the blur behind the sidebar and editor.
        cx.new(|cx| gpui_kit::base::Root::new(workspace, window, cx).bg(transparent_black()))
    })
}
