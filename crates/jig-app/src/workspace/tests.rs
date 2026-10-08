//! Headless UI tests: each opens a real window on GPUI's test platform.
//! The helpers here are shared; the tests are grouped by feature below.

mod agent;
mod code;
mod debug;
mod diagnostics;
mod editing;
mod files;
mod format;
mod git;
mod home;
mod jigs;
mod quick;
mod tabs;

use std::path::Path;
use std::sync::Arc;

use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    AnyWindowHandle, AppContext, Bounds, Entity, Focusable, Point, TestAppContext, WindowBounds,
    WindowOptions, px, size,
};
use jig_ai::Provider;
use jig_commands::{Bubble, ChatEntry};
use jig_editor::EditorHandle;

use super::{Workspace, key_bindings};

/// The editor's move to the end of the text.
const END_OF_FILE: &str = if cfg!(target_os = "macos") {
    "cmd-down"
} else {
    "ctrl-end"
};

/// Answers every request with a fixed reply.
struct FakeProvider(Result<&'static str, &'static str>);

impl Provider for FakeProvider {
    fn complete(&self, _: &str, user: &str) -> anyhow::Result<String> {
        assert!(user.contains("<<<SELECTION>>>"), "the target is marked");
        self.0.map(str::to_string).map_err(|e| anyhow::anyhow!(e))
    }
}

fn use_provider(
    cx: &mut TestAppContext,
    workspace: &Entity<Workspace>,
    reply: Result<&'static str, &'static str>,
) {
    cx.update(|cx| {
        workspace.update(cx, |this, _| {
            this.provider = Ok(Arc::new(FakeProvider(reply)))
        })
    });
}

/// Select `range`, then run the preset matching `query` through Cmd+K.
fn run_preset(
    cx: &mut TestAppContext,
    window: AnyWindowHandle,
    workspace: &Entity<Workspace>,
    range: std::ops::Range<usize>,
    query: &str,
) {
    let workspace = workspace.clone();
    let query = query.to_string();
    step(cx, window, move |window, cx| {
        window.render_frame(cx);
        workspace.update(cx, |this, cx| {
            this.editor()
                .state()
                .update(cx, |s, cx| s.set_selected_range(range, cx))
        });
        window.press("secondary-k", cx);
    });
    step(cx, window, move |window, cx| {
        window.render_frame(cx);
        window.input(&query, cx);
        window.press("enter", cx);
    });
}

fn bubble(cx: &mut TestAppContext, workspace: &Entity<Workspace>) -> Option<Bubble> {
    cx.update(|cx| {
        workspace
            .read(cx)
            .run
            .as_ref()
            .map(|run| run.bubble.clone())
    })
}

fn open(cx: &mut TestAppContext, path: &Path) -> (AnyWindowHandle, Entity<Workspace>) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        cx.bind_keys(key_bindings());
    });
    let path = path.to_path_buf();
    let handles = cx.update(|cx| {
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds {
                origin: Point::default(),
                size: size(px(800.), px(480.)),
            })),
            ..Default::default()
        };
        gpui_kit::open_window(options, cx, |window, cx| {
            cx.new(|cx| Workspace::new(Some(path), window, cx))
        })
        .unwrap()
    });
    cx.run_until_parked();
    handles
}

/// Run `f` in the window, then let subscriptions and effects settle.
fn step(
    cx: &mut TestAppContext,
    window: AnyWindowHandle,
    f: impl FnOnce(&mut gpui_kit::Window, &mut gpui_kit::App),
) {
    cx.update_window(window, |_, window, cx| f(window, cx))
        .unwrap();
    cx.run_until_parked();
}

fn reload_changed_files(
    cx: &mut TestAppContext,
    window: AnyWindowHandle,
    workspace: &Entity<Workspace>,
) {
    step(cx, window, |window, cx| {
        workspace.update(cx, |this, cx| this.reload_changed_files(None, window, cx))
    });
}

const DOCS_REPLY: &str =
    r#"{"replace": "/// Does a.\nfn a() {}", "message": "Added a doc comment."}"#;

const ORIGINAL: &str = "fn a() {}\n";

const DOCUMENTED: &str = "/// Does a.\nfn a() {}\n";

/// Open `lib.rs`, run "Add docs" on `fn a() {}` and wait for the preview.
fn preview_docs(
    cx: &mut TestAppContext,
) -> (tempfile::TempDir, AnyWindowHandle, Entity<Workspace>) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("lib.rs");
    std::fs::write(&path, ORIGINAL).unwrap();
    let (window, workspace) = open(cx, &path);
    use_provider(cx, &workspace, Ok(DOCS_REPLY));
    run_preset(cx, window, &workspace, 0..9, "docs");
    (dir, window, workspace)
}

fn text(cx: &mut TestAppContext, workspace: &Entity<Workspace>) -> String {
    cx.update(|cx| workspace.read(cx).editor().text(cx))
}

/// A workspace whose commands file lives in a temp dir.
fn open_with_commands(
    cx: &mut TestAppContext,
) -> (
    tempfile::TempDir,
    AnyWindowHandle,
    Entity<Workspace>,
    std::path::PathBuf,
) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("lib.rs");
    std::fs::write(&path, ORIGINAL).unwrap();
    let (window, workspace) = open(cx, &path);
    let commands = dir.path().join("config").join("jigs.toml");
    let commands_for_ws = commands.clone();
    cx.update(|cx| workspace.update(cx, |this, _| this.commands_path = Some(commands_for_ws)));
    (dir, window, workspace, commands)
}

fn preset_names(cx: &mut TestAppContext, workspace: &Entity<Workspace>) -> Vec<String> {
    cx.update(|cx| {
        workspace
            .read(cx)
            .presets
            .iter()
            .map(|p| p.name.clone())
            .collect()
    })
}

/// Records the user message of each request.
struct RecordingProvider(std::sync::Mutex<Vec<String>>);

impl Provider for RecordingProvider {
    fn complete(&self, _: &str, user: &str) -> anyhow::Result<String> {
        self.0.lock().unwrap().push(user.to_string());
        Ok(r#"{"replace": "struct User;", "message": "Added."}"#.into())
    }
}

fn tree_focused(cx: &mut TestAppContext, window: AnyWindowHandle, ws: &Entity<Workspace>) -> bool {
    let ws = ws.clone();
    cx.update_window(window, move |_, window, cx| {
        ws.read(cx)
            .tree
            .as_ref()
            .unwrap()
            .view
            .read(cx)
            .is_focused(window)
    })
    .unwrap()
}

fn find_results(cx: &mut TestAppContext, workspace: &Entity<Workspace>) -> Vec<String> {
    cx.update(|cx| {
        let open = workspace.read(cx).find_in_files.as_ref().unwrap();
        let view = open.view.read(cx);
        assert!(!view.is_searching());
        view.result_lines()
    })
}

fn open_file(
    cx: &mut TestAppContext,
    window: AnyWindowHandle,
    workspace: &Entity<Workspace>,
    path: &Path,
) {
    let ws = workspace.clone();
    let path = path.to_path_buf();
    step(cx, window, move |window, cx| {
        ws.update(cx, |this, cx| this.open_file(&path, window, cx))
    });
}

/// `a.rs` open with `b.rs` and `c.rs` next to it on disk.
fn three_files(cx: &mut TestAppContext) -> (tempfile::TempDir, AnyWindowHandle, Entity<Workspace>) {
    let dir = tempfile::tempdir().unwrap();
    for name in ["a.rs", "b.rs", "c.rs"] {
        std::fs::write(dir.path().join(name), format!("// {name}\n")).unwrap();
    }
    let (window, workspace) = open(cx, &dir.path().join("a.rs"));
    (dir, window, workspace)
}

/// Type `text` a character at a time, as completion only follows typing.
fn type_slowly(cx: &mut TestAppContext, window: AnyWindowHandle, text: &str) {
    for c in text.chars() {
        step(cx, window, move |window, cx| {
            window.render_frame(cx);
            window.input(&c.to_string(), cx);
        });
    }
}

fn press(cx: &mut TestAppContext, window: AnyWindowHandle, key: &'static str) {
    step(cx, window, move |window, cx| {
        window.render_frame(cx);
        window.press(key, cx);
    });
}

/// Let the process threads and the UI catch up until `done`.
fn wait_until(cx: &mut TestAppContext, mut done: impl FnMut(&mut TestAppContext) -> bool) {
    for _ in 0..500 {
        cx.executor()
            .advance_clock(std::time::Duration::from_millis(50));
        cx.run_until_parked();
        if done(cx) {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    panic!("timed out");
}

fn selection(cx: &mut TestAppContext, workspace: &Entity<Workspace>) -> std::ops::Range<usize> {
    cx.update(|cx| workspace.read(cx).editor().selection(cx))
}
