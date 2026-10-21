//! Headless UI tests: each opens a real window on GPUI's test platform.
//! The helpers here are shared; the tests are grouped by feature below.

mod agent;
mod brackets;
mod code;
mod debug;
mod diagnostics;
mod domains;
mod editing;
mod files;
mod format;
mod git;
mod home;
mod input;
mod lens;
mod map;
mod occurrences;
mod quick_fix;
mod servers;
mod session;
mod site;
mod tabs;
mod tasks;
mod update;

use std::path::Path;

use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    AnyWindowHandle, AppContext, Bounds, Entity, Focusable, Point, TestAppContext, WindowBounds,
    WindowOptions, px, size,
};
use jig_commands::{Bubble, ChatEntry};
use jig_editor::EditorHandle;

use super::{Workspace, key_bindings};

/// The editor's move to the end of the text.
const END_OF_FILE: &str = if cfg!(target_os = "macos") {
    "cmd-down"
} else {
    "ctrl-end"
};

/// The note beside the code, if one is showing.
fn note(cx: &mut TestAppContext, workspace: &Entity<Workspace>) -> Option<Bubble> {
    cx.update(|cx| {
        workspace
            .read(cx)
            .note
            .as_ref()
            .map(|note| note.bubble.clone())
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

const ORIGINAL: &str = "fn a() {}\n";

const DOCUMENTED: &str = "/// Does a.\nfn a() {}\n";

/// As if the agent, in a conversation without OpenCode behind it, wanted
/// the file on screen to read `new`: the edit waits under review.
fn review_agent_edit(
    cx: &mut TestAppContext,
    window: AnyWindowHandle,
    workspace: &Entity<Workspace>,
    new: &str,
) {
    let (ws, new) = (workspace.clone(), new.to_string());
    step(cx, window, move |window, cx| {
        ws.update(cx, |this, cx| {
            let path = this.document().path.clone().expect("a saved file");
            this.open_test_conversation(window, cx);
            let edit = jig_ai::agent::EditRequest {
                id: "per_edit".into(),
                path,
                change: jig_ai::agent::EditChange::Write(new),
            };
            this.test_agent_request(jig_ai::agent::AgentEvent::Edit(edit), window, cx);
        })
    });
    assert!(cx.update(|cx| workspace.read(cx).previewing()));
}

/// Open `lib.rs` with `fn a() {}`, and have the agent want to add a doc
/// comment to it.
fn preview_docs(
    cx: &mut TestAppContext,
) -> (tempfile::TempDir, AnyWindowHandle, Entity<Workspace>) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("lib.rs");
    std::fs::write(&path, ORIGINAL).unwrap();
    let (window, workspace) = open(cx, &path);
    review_agent_edit(cx, window, &workspace, DOCUMENTED);
    (dir, window, workspace)
}

fn text(cx: &mut TestAppContext, workspace: &Entity<Workspace>) -> String {
    cx.update(|cx| workspace.read(cx).editor().text(cx))
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
