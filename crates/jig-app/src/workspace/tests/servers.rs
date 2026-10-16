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
