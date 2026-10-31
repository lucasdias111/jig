//! Language servers, as Neovim runs them: a table saying which server
//! serves which languages and which files mark a project's root, one server
//! per root shared by every window, and documents kept in sync as they're
//! edited.
//!
//! Servers aren't bundled. One that isn't installed is simply not used, and
//! Cmd+click falls back to [`crate::definitions`].

mod client;
pub mod diagnostics;
pub mod install;

use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use crate::languages::{Server, program_for};
use crate::tools::search_path;
use gpui_kit::{App, Global};
use jig_process::NoWindow as _;

pub use client::{Client, Response};

/// How a server counts the columns in a position.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Encoding {
    Utf8,
    /// The protocol's default.
    Utf16,
}

/// The server for `language`, if there is one.
pub fn server_for(language: &str) -> Option<&'static Server> {
    crate::languages::servers()
        .iter()
        .find(|s| s.languages.iter().any(|l| l == language))
}

/// The protocol's name for a language.
pub fn language_id(language: &str, path: &Path) -> &'static str {
    let jsx = path.extension().is_some_and(|e| e == "jsx");
    match crate::languages::get(language) {
        Some(_) if language == "javascript" && jsx => "javascriptreact",
        Some(known) => known.lsp_id,
        None => "plaintext",
    }
}

/// The root `server` should run in for `file`: the folder with one of its
/// markers, else the project root (the nearest `.git`).
pub fn root_for(server: &Server, file: &Path) -> PathBuf {
    let dir = file.parent().unwrap_or(Path::new("/"));
    let mut marked = dir.ancestors().filter(|dir| server.marks(dir));
    let found = if server.outermost {
        marked.last()
    } else {
        marked.next()
    };
    found.map_or_else(|| crate::project::root_for(file), Path::to_path_buf)
}

/// A server and the root it runs in.
type Key = (&'static str, PathBuf);

/// The running servers, one per server and root.
#[derive(Default)]
pub struct Registry {
    clients: HashMap<Key, Arc<Client>>,
    /// Servers started in a folder without their markers, as when a file
    /// opens in a project before `cargo init` has run. rust-analyzer never
    /// finds a project that appears after it starts, so these are started
    /// again once their markers show up ([`Registry::outgrown`]).
    unmarked: HashSet<Key>,
    /// Where every server's problems go, on their way to
    /// [`diagnostics::Diagnostics`].
    diagnostics: Option<futures::channel::mpsc::UnboundedSender<diagnostics::Published>>,
    /// Where servers say they stopped before they started, on their way to
    /// [`Failures`].
    failures: Option<futures::channel::mpsc::UnboundedSender<()>>,
}

impl Global for Registry {}

impl Registry {
    /// Whether the server at `key` started without its markers and its root
    /// has them now.
    fn outgrown(&self, server: &Server, key: &Key) -> bool {
        self.unmarked.contains(key) && server.marks(&key.1)
    }

    fn insert(&mut self, server: &Server, key: Key, client: Arc<Client>) {
        if server.marks(&key.1) {
            self.unmarked.remove(&key);
        } else {
            self.unmarked.insert(key.clone());
        }
        self.clients.insert(key, client);
    }
}

/// Bumped when a server stops before it has started, for workspaces to say
/// why ([`Client::failure`]) rather than quietly go without it.
#[derive(Default)]
pub struct Failures(pub u64);

impl Global for Failures {}

/// Use `client` for `server`'s files under `root`, for tests.
#[cfg(test)]
pub fn register(server: &'static str, root: PathBuf, client: Arc<Client>, cx: &mut App) {
    client.on_diagnostics(diagnostics_sink(cx));
    client.on_failure(failures_sink(cx));
    let server = crate::languages::server_named(server).expect("a known server");
    cx.default_global::<Registry>()
        .insert(server, (&server.name, root), client);
}

/// The channel servers' threads send problems down, read on the main thread
/// into [`diagnostics::Diagnostics`].
fn diagnostics_sink(
    cx: &mut App,
) -> futures::channel::mpsc::UnboundedSender<diagnostics::Published> {
    if let Some(sink) = &cx.default_global::<Registry>().diagnostics {
        return sink.clone();
    }
    let (sink, mut reports) = futures::channel::mpsc::unbounded();
    cx.spawn(async move |cx| {
        use futures::StreamExt as _;
        while let Some(published) = reports.next().await {
            cx.update(|cx| diagnostics::publish(published, cx));
        }
    })
    .detach();
    cx.default_global::<Registry>().diagnostics = Some(sink.clone());
    sink
}

/// The channel servers' threads say they failed down, read on the main
/// thread into [`Failures`].
fn failures_sink(cx: &mut App) -> futures::channel::mpsc::UnboundedSender<()> {
    if let Some(sink) = &cx.default_global::<Registry>().failures {
        return sink.clone();
    }
    let (sink, mut failures) = futures::channel::mpsc::unbounded();
    cx.spawn(async move |cx| {
        use futures::StreamExt as _;
        while failures.next().await.is_some() {
            cx.update(|cx| cx.default_global::<Failures>().0 += 1);
        }
    })
    .detach();
    cx.default_global::<Registry>().failures = Some(sink.clone());
    sink
}

/// The client for `file`, starting its server if it isn't running. `None`
/// when no server serves `language` or none is installed.
pub fn client_for(file: &Path, language: &str, cx: &mut App) -> Option<Arc<Client>> {
    let server = server_for(language)?;
    let file = std::path::absolute(file).ok()?;
    let root = root_for(server, &file);
    let key = (server.name.as_str(), root.clone());
    let registry = cx.default_global::<Registry>();
    if registry.outgrown(server, &key) {
        // Its tabs let go of it as they move to the new one, and the last
        // to go shuts it down.
        registry.clients.remove(&key);
    } else if let Some(client) = registry.clients.get(&key) {
        return (!client.is_dead()).then(|| client.clone());
    }
    // Tests use fake servers, never what happens to be installed, unless
    // a live test asks for the real ones.
    if cfg!(test) && std::env::var_os("JIG_LIVE_LSP").is_none() {
        return None;
    }
    // One that isn't installed is kept too, dead, so it isn't looked for
    // again.
    let command_root = root.clone();
    let init_options = init_options(server);
    let client = Client::start(&server.name, root, init_options, move || {
        command_for(server, &command_root)
    });
    registry.insert(server, key, client.clone());
    client.on_diagnostics(diagnostics_sink(cx));
    client.on_failure(failures_sink(cx));
    (!client.is_dead()).then_some(client)
}

/// Whether `client` is still the one [`client_for`] gives `file`: not when
/// a marker has since appeared, giving the file another root or its
/// server's root the markers it started without.
pub fn is_current(client: &Arc<Client>, file: &Path, language: &str, cx: &App) -> bool {
    let Some(server) = server_for(language) else {
        return false;
    };
    let Ok(file) = std::path::absolute(file) else {
        return true;
    };
    let key = (server.name.as_str(), root_for(server, &file));
    cx.try_global::<Registry>().is_some_and(|registry| {
        !registry.outgrown(server, &key)
            && registry
                .clients
                .get(&key)
                .is_some_and(|current| Arc::ptr_eq(current, client))
    })
}

/// Why `file`'s server stopped before it started, if it did.
pub fn failure_for(file: &Path, language: &str, cx: &App) -> Option<String> {
    let server = server_for(language)?;
    let root = root_for(server, &std::path::absolute(file).ok()?);
    cx.try_global::<Registry>()?
        .clients
        .get(&(server.name.as_str(), root))?
        .failure()
}

/// Bumped when a server is installed or uninstalled, for workspaces to
/// open their files in it again.
#[derive(Default)]
pub struct ServersChanged(pub u64);

impl Global for ServersChanged {}

/// A server was installed or uninstalled: forget the ones that weren't
/// found when last looked for, and tell the workspaces.
pub fn servers_changed(cx: &mut App) {
    let Registry {
        clients, unmarked, ..
    } = cx.default_global::<Registry>();
    clients.retain(|_, client| !client.is_dead());
    unmarked.retain(|key| clients.contains_key(key));
    cx.default_global::<ServersChanged>().0 += 1;
}

/// What `server` is told in `initialize`: for jdtls, the plugins debuggers
/// ask it to load, such as java-debug.
fn init_options(server: &Server) -> serde_json::Value {
    let bundles = crate::debuggers::lsp_bundles(&server.name);
    if bundles.is_empty() {
        return serde_json::Value::Null;
    }
    serde_json::json!({"bundles": bundles})
}

/// The command to run `server` with, if one of its commands is installed.
fn command_for(server: &Server, root: &Path) -> Option<Command> {
    let (program, args) = program_for(server)?;
    let mut command = Command::new(program);
    command.no_window().args(args).env("PATH", search_path());
    if server.name == "jdtls" {
        command.arg("-data").arg(jdtls_data(root));
    }
    Some(command)
}

/// jdtls keeps an index per project; somewhere out of the project.
fn jdtls_data(root: &Path) -> PathBuf {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    root.hash(&mut hasher);
    cache_dir()
        .join("Jig/jdtls")
        .join(format!("{:016x}", hasher.finish()))
}

/// Where this OS keeps per-user caches, or the temp folder if that's unknown.
fn cache_dir() -> PathBuf {
    let var = |name| {
        std::env::var_os(name)
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
    };
    // Live tests keep their indexes out of the real caches.
    if let Some(dir) = var("JIG_CACHE_DIR") {
        return dir;
    }
    let dir = if cfg!(target_os = "macos") {
        var("HOME").map(|home| home.join("Library/Caches"))
    } else if cfg!(windows) {
        var("LOCALAPPDATA")
    } else {
        var("XDG_CACHE_HOME").or_else(|| var("HOME").map(|home| home.join(".cache")))
    };
    dir.unwrap_or_else(std::env::temp_dir)
}

/// The LSP position of byte `offset` in `text`.
pub fn position(text: &str, offset: usize, encoding: Encoding) -> lsp_types::Position {
    let offset = offset.min(text.len());
    let line_start = text[..offset].rfind('\n').map_or(0, |ix| ix + 1);
    let line = text[..line_start].matches('\n').count();
    let column = &text[line_start..offset];
    let character = match encoding {
        Encoding::Utf8 => column.len(),
        Encoding::Utf16 => column.encode_utf16().count(),
    };
    lsp_types::Position::new(line as u32, character as u32)
}

/// The byte offset of an LSP position in `text`, clamped to its line.
pub fn offset(text: &str, position: lsp_types::Position, encoding: Encoding) -> usize {
    let mut line_start = 0;
    for _ in 0..position.line {
        match text[line_start..].find('\n') {
            Some(ix) => line_start += ix + 1,
            None => return text.len(),
        }
    }
    let line_end = text[line_start..]
        .find('\n')
        .map_or(text.len(), |ix| line_start + ix);
    let line = &text[line_start..line_end];
    let wanted = position.character as usize;
    let mut units = 0;
    for (ix, c) in line.char_indices() {
        if units >= wanted {
            return line_start + ix;
        }
        units += match encoding {
            Encoding::Utf8 => c.len_utf8(),
            Encoding::Utf16 => c.len_utf16(),
        };
    }
    line_end
}

pub fn range(text: &str, range: lsp_types::Range, encoding: Encoding) -> Range<usize> {
    offset(text, range.start, encoding)..offset(text, range.end, encoding)
}

/// A `file:` URI for `path`, made absolute first: a relative path makes no
/// sense to a server.
pub fn file_uri(path: &Path) -> String {
    let path = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
    let path = path.to_string_lossy();
    // `C:\a\b.rs` is `file:///C:/a/b.rs`, the drive's colon kept as is.
    if cfg!(windows) {
        let path = path.strip_prefix(r"\\?\").unwrap_or(&path);
        let path = format!("/{}", path.replace('\\', "/"));
        let mut encoded = encode(&path);
        if encoded.get(2..5) == Some("%3A") {
            encoded.replace_range(2..5, ":");
        }
        return format!("file://{encoded}");
    }
    format!("file://{}", encode(&path))
}

/// The path of a `file:` URI.
pub fn uri_path(uri: &str) -> Option<PathBuf> {
    let path = uri.strip_prefix("file://")?;
    // `file://localhost/…` is allowed, if rare.
    let path = path.strip_prefix("localhost").unwrap_or(path);
    let path = decode(path);
    // `/C:/a/b.rs` on Windows, where the drive comes first.
    if cfg!(windows)
        && let Some(rest) = path.strip_prefix('/')
        && rest.as_bytes().get(1) == Some(&b':')
    {
        return Some(PathBuf::from(rest));
    }
    Some(PathBuf::from(path))
}

/// Percent-encode everything but unreserved characters and `/`.
pub fn encode(text: &str) -> String {
    text.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

pub fn decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = (bytes[i] == b'%')
            .then(|| text.get(i + 1..i + 3))
            .flatten()
            .and_then(|hex| u8::from_str_radix(hex, 16).ok());
        match hex {
            Some(byte) => {
                out.push(byte);
                i += 3;
            }
            None => {
                out.push(bytes[i]);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Where a definition or reference request pointed: a file and a range in
/// the server's positions.
pub fn locations(response: &serde_json::Value) -> Vec<(PathBuf, lsp_types::Range)> {
    let items = match response {
        serde_json::Value::Array(items) => items.clone(),
        serde_json::Value::Null => Vec::new(),
        single => vec![single.clone()],
    };
    items
        .iter()
        .filter_map(|item| {
            // A `Location`, or a `LocationLink` pointing at its name.
            let (uri, range) = match item.get("targetUri") {
                Some(uri) => (uri, item.get("targetSelectionRange")?),
                None => (item.get("uri")?, item.get("range")?),
            };
            let path = uri_path(uri.as_str()?)?;
            let range = serde_json::from_value(range.clone()).ok()?;
            Some((path, range))
        })
        .collect()
}

pub fn text_document_position(uri: &str, position: lsp_types::Position) -> serde_json::Value {
    serde_json::json!({
        "textDocument": {"uri": uri},
        "position": position,
    })
}

#[cfg(test)]
pub use client::tests::fake_server;

#[cfg(test)]
mod tests {
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
        let range =
            json!({"start": {"line": 1, "character": 2}, "end": {"line": 1, "character": 5}});
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
            let mut params =
                text_document_position(&lib_uri, position(lib, call, client.encoding()));
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
}
