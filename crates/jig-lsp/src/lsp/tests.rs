use serde_json::json;

use super::*;

#[test]
fn positions_round_trip_in_both_encodings() {
    let text = "fn a() {}\nlet é = \"😀\"; x\n";
    let x = text.rfind('x').unwrap();
    assert_eq!(
        position(text, x, Encoding::Utf8),
        lsp_types::Position::new(1, 17)
    );
    assert_eq!(
        position(text, x, Encoding::Utf16),
        lsp_types::Position::new(1, 14)
    );
    for encoding in [Encoding::Utf8, Encoding::Utf16] {
        for offset in [0, 3, 10, x, text.len()] {
            assert_eq!(offset_of(text, offset, encoding), offset);
        }
    }
    // Past the end of a line stays on it.
    assert_eq!(
        offset(text, lsp_types::Position::new(0, 99), Encoding::Utf8),
        9
    );
    assert_eq!(
        offset(text, lsp_types::Position::new(9, 0), Encoding::Utf8),
        text.len()
    );
}

fn offset_of(text: &str, offset: usize, encoding: Encoding) -> usize {
    super::offset(text, position(text, offset, encoding), encoding)
}

#[test]
fn uris_round_trip() {
    let (path, uri, expected) = if cfg!(windows) {
        (
            r"C:\Users\me\My Projects\naïve#1.rs",
            "file:///C:/Users/me/a%20b.rs",
            r"C:\Users\me\a b.rs",
        )
    } else {
        (
            "/Users/me/My Projects/naïve#1.rs",
            "file:///Users/me/a%20b.rs",
            "/Users/me/a b.rs",
        )
    };
    let path = Path::new(path);
    assert_eq!(uri_path(&file_uri(path)).unwrap(), path);
    assert_eq!(uri_path(uri).unwrap(), Path::new(expected));
    if cfg!(windows) {
        assert!(file_uri(path).starts_with("file:///C:/Users/me/My%20Projects/"));
    }
}

#[test]
fn locations_and_links() {
    let range = json!({"start": {"line": 1, "character": 2}, "end": {"line": 1, "character": 5}});
    let expected = (
        PathBuf::from("/p/a.rs"),
        lsp_types::Range::new(
            lsp_types::Position::new(1, 2),
            lsp_types::Position::new(1, 5),
        ),
    );
    let location = json!({"uri": "file:///p/a.rs", "range": range});
    let link = json!({
        "targetUri": "file:///p/a.rs",
        "targetRange": {"start": {"line": 0, "character": 0}, "end": {"line": 3, "character": 0}},
        "targetSelectionRange": range,
    });
    assert_eq!(locations(&location), std::slice::from_ref(&expected));
    assert_eq!(locations(&json!([link])), [expected]);
    assert!(locations(&serde_json::Value::Null).is_empty());
}

#[test]
fn roots_follow_markers() {
    let dir = tempfile::tempdir().unwrap();
    let dir = dir.path().canonicalize().unwrap();
    std::fs::create_dir_all(dir.join("crates/a/src")).unwrap();
    std::fs::write(dir.join("Cargo.toml"), "").unwrap();
    std::fs::write(dir.join("crates/a/Cargo.toml"), "").unwrap();
    std::fs::write(dir.join("crates/a/package.json"), "").unwrap();
    let file = dir.join("crates/a/src/lib.rs");
    let rust = server_for("rust").unwrap();
    let ts = server_for("typescript").unwrap();
    assert_eq!(root_for(rust, &file), dir, "the Cargo workspace");
    assert_eq!(
        root_for(ts, &file),
        dir.join("crates/a"),
        "the nearest package"
    );
}

/// Against the installed rust-analyzer, if there is one:
/// `cargo test -p jig-app real_rust_analyzer -- --ignored`.
#[test]
#[ignore]
fn real_rust_analyzer() {
    use futures::executor::block_on;

    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(
        root.join("Cargo.toml"),
        "[package]\nname = \"demo\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    let lib = "pub mod util;\npub fn run() -> u32 {\n    util::helper()\n}\n";
    let util = "pub fn helper() -> u32 {\n    1\n}\n";
    std::fs::write(root.join("src/lib.rs"), lib).unwrap();
    std::fs::write(root.join("src/util.rs"), util).unwrap();

    let server = server_for("rust").unwrap();
    let command_root = root.clone();
    let client = Client::start(
        &server.name,
        root.clone(),
        serde_json::Value::Null,
        move || command_for(server, &command_root),
    );
    let lib_uri = file_uri(&root.join("src/lib.rs"));
    client.open(&lib_uri, "rust", lib);
    let call = lib.find("helper").unwrap();

    // It answers once it has loaded the project; ask until it does.
    let started = std::time::Instant::now();
    let found = loop {
        assert!(!client.is_dead(), "rust-analyzer exited");
        assert!(started.elapsed().as_secs() < 120, "no answer in 2 minutes");
        let position = position(lib, call, client.encoding());
        let response = block_on(client.request(
            "textDocument/definition",
            text_document_position(&lib_uri, position),
        ));
        if let Ok(Ok(value)) = response {
            let found = locations(&value);
            if !found.is_empty() {
                break found;
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(500));
    };
    eprintln!("definition after {:?}: {found:?}", started.elapsed());
    let (path, target) = &found[0];
    assert_eq!(path.canonicalize().unwrap(), root.join("src/util.rs"));
    assert_eq!(&util[range(util, *target, client.encoding())], "helper");

    // References can take longer: they need the whole crate indexed.
    let mut files: Vec<String> = loop {
        assert!(
            started.elapsed().as_secs() < 120,
            "no references in 2 minutes"
        );
        let mut params = text_document_position(&lib_uri, position(lib, call, client.encoding()));
        params["context"] = serde_json::json!({"includeDeclaration": true});
        let references = block_on(client.request("textDocument/references", params))
            .unwrap()
            .unwrap();
        let found = locations(&references);
        if !found.is_empty() {
            eprintln!("references after {:?}: {found:?}", started.elapsed());
            break found
                .iter()
                .map(|(path, _)| path.file_name().unwrap().to_string_lossy().into_owned())
                .collect();
        }
        std::thread::sleep(std::time::Duration::from_millis(250));
    };
    files.sort();
    assert_eq!(files, ["lib.rs", "util.rs"]);
}
