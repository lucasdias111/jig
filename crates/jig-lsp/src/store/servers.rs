//! Which server serves a file, the root it runs in, and the command and
//! options it's started with.

use std::path::{Path, PathBuf};
use std::process::Command;

use jig_languages::{Server, program_for};
use jig_process::NoWindow as _;
use jig_tools::search_path;

/// The server for `language`, if there is one.
pub fn server_for(language: &str) -> Option<&'static Server> {
    jig_languages::servers()
        .iter()
        .find(|s| s.languages.iter().any(|l| l == language))
}

/// The protocol's name for a language.
pub fn language_id(language: &str, path: &Path) -> &'static str {
    let jsx = path.extension().is_some_and(|e| e == "jsx");
    match jig_languages::get(language) {
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
    found.map_or_else(|| jig_kit::project::root_for(file), Path::to_path_buf)
}

/// What `server` is told in `initialize`: for jdtls, the plugins debuggers
/// ask it to load, such as java-debug.
pub(crate) fn init_options(server: &Server) -> serde_json::Value {
    let bundles = jig_debuggers::lsp_bundles(&server.name);
    if bundles.is_empty() {
        return serde_json::Value::Null;
    }
    serde_json::json!({"bundles": bundles})
}

/// The command to run `server` with, if one of its commands is installed.
pub(crate) fn command_for(server: &Server, root: &Path) -> Option<Command> {
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
