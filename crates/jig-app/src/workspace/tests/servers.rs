//! Which language server each tab's file is open in.

use super::*;

/// A file opened before `cargo init` leaves the server started without a
/// `Cargo.toml` once one appears, as rust-analyzer never finds a project
/// made after it starts.
#[gpui_kit::test]
fn tabs_leave_a_server_started_before_its_project(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    std::fs::create_dir(root.join(".git")).unwrap();
    std::fs::create_dir(root.join("src")).unwrap();
    let path = root.join("src/main.rs");
    std::fs::write(&path, "fn main() {}\n").unwrap();
    let server = crate::lsp::server_for("rust").unwrap();
    assert_eq!(crate::lsp::root_for(server, &path), root, "the repository");
    let (client, seen) = crate::lsp::fake_server(root.clone(), |_, _| serde_json::Value::Null);
    cx.update(|cx| crate::lsp::register(&server.name, root.clone(), client, cx));
    let (window, workspace) = open(cx, &root);
    open_file(cx, window, &workspace, &path);
    let in_a_server =
        |cx: &mut TestAppContext| cx.update(|cx| workspace.read(cx).tab().lsp.is_some());
    let check = |cx: &mut TestAppContext| {
        let ws = workspace.clone();
        step(cx, window, move |_, cx| {
            ws.update(cx, |this, cx| this.move_to_current_servers(cx))
        });
    };
    assert!(in_a_server(cx));
    check(cx);
    assert!(in_a_server(cx), "nothing has changed");

    std::fs::write(root.join("Cargo.toml"), "[package]\nname = \"demo\"\n").unwrap();
    check(cx);
    // Tests start no real servers, so it's in none now.
    assert!(!in_a_server(cx));
    let closed = std::iter::from_fn(|| seen.recv_timeout(std::time::Duration::from_secs(5)).ok())
        .any(|message| message["method"] == "textDocument/didClose");
    assert!(closed, "the old server is told the file closed");
}

#[gpui_kit::test]
#[ignore]
fn a_mended_cargo_toml_brings_problems_back_live(cx: &mut TestAppContext) {
    assert!(std::env::var_os("JIG_LIVE_LSP").is_some(), "JIG_LIVE_LSP=1");
    cx.executor().allow_parking();
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let manifest = "[package]\nname = \"sample\"\nversion = \"0.1.0\"\nedition = \"2024\"\n";
    std::fs::create_dir(root.join("src")).unwrap();
    std::fs::write(root.join("Cargo.toml"), manifest).unwrap();
    let main = root.join("src/main.rs");
    std::fs::write(&main, "fn main() {\n    let x: u32 = \"s\";\n}\n").unwrap();
    let (window, workspace) = open(cx, &root);
    open_file(cx, window, &workspace, &main);
    let key = crate::lsp::diagnostics::path_key(&main);
    let problems = |cx: &mut TestAppContext| {
        cx.update(|cx| {
            cx.try_global::<crate::lsp::diagnostics::Diagnostics>()
                .and_then(|store| store.get(&key))
                .map_or(0, |stored| stored.published.items.len())
        })
    };
    let wait = |cx: &mut TestAppContext, seconds: u64, done: &dyn Fn(usize) -> bool| {
        for _ in 0..seconds * 10 {
            std::thread::sleep(std::time::Duration::from_millis(100));
            cx.executor()
                .advance_clock(std::time::Duration::from_millis(50));
            cx.run_until_parked();
            if done(problems(cx)) {
                return true;
            }
        }
        false
    };
    let save_manifest = |cx: &mut TestAppContext, text: &'static str| {
        let ws = workspace.clone();
        step(cx, window, move |window, cx| {
            ws.update(cx, |this, cx| {
                let editor = this.editor().clone();
                let len = editor.text(cx).len();
                editor.apply_edit(0..len, text, window, cx);
                this.save(&crate::workspace::Save, window, cx);
            });
        });
    };
    assert!(wait(cx, 60, &|n| n > 0), "the type error shows");
    open_file(cx, window, &workspace, &root.join("Cargo.toml"));
    save_manifest(cx, "[package\nname = ");
    eprintln!("broken: cleared = {}", wait(cx, 15, &|n| n == 0));
    save_manifest(cx, manifest);
    assert!(wait(cx, 60, &|n| n > 0), "the type error shows again");

    // Back on main.rs, as you'd go: its squiggles and hover are back.
    open_file(cx, window, &workspace, &main);
    let squiggles = cx.update(|cx| {
        let state = workspace.read(cx).editor().state().read(cx);
        state.diagnostics().map_or(0, |set| set.iter().count())
    });
    assert!(squiggles > 0, "the tab draws the problems");
    let (client, uri) = cx.update(|cx| {
        let lsp = workspace.read(cx).tab().lsp.as_ref().expect("in a server");
        (lsp.client.clone(), lsp.uri.clone())
    });
    let hover = futures::executor::block_on(client.request(
        "textDocument/hover",
        serde_json::json!({
            "textDocument": {"uri": uri},
            "position": {"line": 1, "character": 8},
        }),
    ));
    let hover = hover.unwrap().unwrap();
    eprintln!("hover: {hover}");
    assert!(hover.to_string().contains("u32"), "hover answers");
}
