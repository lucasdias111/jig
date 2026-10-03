//! Read-only tools that let the model look around the project before it
//! answers: list a folder, read a file, search for text.
//!
//! Everything stays inside the project root. Ignored files (per
//! `.gitignore`), the `.git` folder and files that commonly hold secrets are
//! never listed, read or searched, and every result is size-capped.

use std::path::{Component, Path, PathBuf};

use ignore::WalkBuilder;
use ignore::gitignore::{Gitignore, GitignoreBuilder};
use serde_json::{Value, json};

/// Most bytes of one file returned by `read_file`.
const MAX_READ_BYTES: usize = 48 * 1024;
/// Files larger than this are skipped by `search`.
const MAX_SEARCH_FILE_BYTES: u64 = 1024 * 1024;
const MAX_SEARCH_MATCHES: usize = 40;
const MAX_LIST_ENTRIES: usize = 200;
const MAX_LINE_CHARS: usize = 200;

/// A tool the model may call.
#[derive(Clone, Debug)]
pub struct ToolSpec {
    pub name: &'static str,
    pub description: &'static str,
    /// JSON Schema for the input object.
    pub input_schema: Value,
}

/// Runs tool calls for a provider's tool loop.
pub trait ToolHost: Send + Sync {
    fn specs(&self) -> Vec<ToolSpec>;
    /// Run `name` with `input` and return the text the model sees. Errors
    /// are returned as text too, so the model can recover.
    fn call(&self, name: &str, input: &Value) -> String;
    /// A short, human-readable line for the UI, e.g. "Reading src/lib.rs".
    fn describe(&self, name: &str, input: &Value) -> String;
}

/// The read-only project tools, rooted at a project folder.
pub struct ProjectTools {
    root: PathBuf,
}

impl ProjectTools {
    pub fn new(root: &Path) -> std::io::Result<Self> {
        Ok(Self {
            root: root.canonicalize()?,
        })
    }

    /// Resolve a model-supplied path to a real path inside the root.
    fn resolve(&self, path: &str) -> Result<PathBuf, String> {
        let relative = Path::new(path.trim().trim_start_matches("./"));
        if relative.is_absolute()
            || relative
                .components()
                .any(|c| matches!(c, Component::ParentDir | Component::Prefix(_)))
        {
            return Err(format!(
                "{path}: use a path relative to the project root, without ..",
            ));
        }
        let full = self.root.join(relative);
        let full = full
            .canonicalize()
            .map_err(|_| format!("{path}: not found"))?;
        if !full.starts_with(&self.root) {
            return Err(format!("{path}: outside the project"));
        }
        Ok(full)
    }

    fn display(&self, path: &Path) -> String {
        let relative = path.strip_prefix(&self.root).unwrap_or(path);
        let text = relative.to_string_lossy().replace('\\', "/");
        if text.is_empty() { ".".into() } else { text }
    }

    /// The `.gitignore` rules that apply to `path`, from the root down.
    fn gitignore_for(&self, path: &Path) -> Gitignore {
        let mut builder = GitignoreBuilder::new(&self.root);
        let dir = if path.is_dir() {
            path
        } else {
            path.parent().unwrap_or(&self.root)
        };
        let mut dirs: Vec<&Path> = dir
            .ancestors()
            .take_while(|d| d.starts_with(&self.root))
            .collect();
        dirs.reverse();
        for dir in dirs {
            let file = dir.join(".gitignore");
            if file.is_file() {
                builder.add(file);
            }
        }
        builder.build().unwrap_or_else(|_| Gitignore::empty())
    }

    fn is_hidden_from_model(&self, path: &Path) -> bool {
        let relative = path.strip_prefix(&self.root).unwrap_or(path);
        if relative.components().any(|c| c.as_os_str() == ".git") {
            return true;
        }
        if path
            .file_name()
            .is_some_and(|name| is_secret_file(&name.to_string_lossy()))
        {
            return true;
        }
        self.gitignore_for(path)
            .matched_path_or_any_parents(path, path.is_dir())
            .is_ignore()
    }

    fn walker(&self, from: &Path) -> WalkBuilder {
        let mut walk = WalkBuilder::new(from);
        walk.hidden(false)
            .git_ignore(true)
            .git_exclude(true)
            .require_git(false)
            .parents(true);
        walk.filter_entry(|entry| {
            let name = entry.file_name().to_string_lossy();
            name != ".git" && !is_secret_file(&name)
        });
        walk
    }

    fn list_dir(&self, input: &Value) -> Result<String, String> {
        let path = input["path"].as_str().unwrap_or(".");
        let dir = self.resolve(path)?;
        if !dir.is_dir() {
            return Err(format!("{path}: not a folder"));
        }
        if self.is_hidden_from_model(&dir) && dir != self.root {
            return Err(format!("{path}: not available"));
        }
        let mut entries: Vec<String> = self
            .walker(&dir)
            .max_depth(Some(1))
            .build()
            .filter_map(Result::ok)
            .filter(|entry| entry.path() != dir)
            .map(|entry| {
                let name = self.display(entry.path());
                if entry.file_type().is_some_and(|t| t.is_dir()) {
                    format!("{name}/")
                } else {
                    name
                }
            })
            .collect();
        entries.sort();
        let total = entries.len();
        entries.truncate(MAX_LIST_ENTRIES);
        if total > MAX_LIST_ENTRIES {
            entries.push(format!("… {} more", total - MAX_LIST_ENTRIES));
        }
        Ok(if entries.is_empty() {
            "(empty)".into()
        } else {
            entries.join("\n")
        })
    }

    fn read_file(&self, input: &Value) -> Result<String, String> {
        let path = input["path"].as_str().ok_or("read_file needs a path")?;
        let file = self.resolve(path)?;
        if !file.is_file() {
            return Err(format!("{path}: not a file"));
        }
        if self.is_hidden_from_model(&file) {
            return Err(format!(
                "{path}: not available (ignored or may hold secrets)"
            ));
        }
        let bytes = std::fs::read(&file).map_err(|error| format!("{path}: {error}"))?;
        if bytes.contains(&0) {
            return Err(format!("{path}: binary file"));
        }
        let text = String::from_utf8_lossy(&bytes);
        if text.len() <= MAX_READ_BYTES {
            return Ok(text.into_owned());
        }
        let mut end = MAX_READ_BYTES;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        Ok(format!(
            "{}\n[cut: the file is longer than {MAX_READ_BYTES} bytes]",
            &text[..end]
        ))
    }

    fn search(&self, input: &Value) -> Result<String, String> {
        let query = input["query"].as_str().map(str::trim).unwrap_or_default();
        if query.is_empty() {
            return Err("search needs a query".into());
        }
        let from = self.resolve(input["path"].as_str().unwrap_or("."))?;
        let needle = query.to_lowercase();
        let mut matches = Vec::new();
        'files: for entry in self.walker(&from).build().filter_map(Result::ok) {
            let path = entry.path();
            if !entry.file_type().is_some_and(|t| t.is_file())
                || entry
                    .metadata()
                    .map_or(true, |m| m.len() > MAX_SEARCH_FILE_BYTES)
            {
                continue;
            }
            let Ok(bytes) = std::fs::read(path) else {
                continue;
            };
            if bytes.contains(&0) {
                continue;
            }
            for (number, line) in String::from_utf8_lossy(&bytes).lines().enumerate() {
                if line.to_lowercase().contains(&needle) {
                    let line: String = line.trim().chars().take(MAX_LINE_CHARS).collect();
                    matches.push(format!("{}:{}: {line}", self.display(path), number + 1));
                    if matches.len() >= MAX_SEARCH_MATCHES {
                        matches.push(format!("[stopped after {MAX_SEARCH_MATCHES} matches]"));
                        break 'files;
                    }
                }
            }
        }
        Ok(if matches.is_empty() {
            format!("No matches for “{query}”.")
        } else {
            matches.join("\n")
        })
    }
}

impl ToolHost for ProjectTools {
    fn specs(&self) -> Vec<ToolSpec> {
        vec![
            ToolSpec {
                name: "list_dir",
                description: "List the files and folders in a project folder. Folders end with /.",
                input_schema: json!({
                    "type": "object",
                    "properties": { "path": { "type": "string", "description": "Folder relative to the project root, e.g. \"src\". Defaults to the root." } },
                }),
            },
            ToolSpec {
                name: "read_file",
                description: "Read a text file in the project.",
                input_schema: json!({
                    "type": "object",
                    "properties": { "path": { "type": "string", "description": "File relative to the project root, e.g. \"src/models/user.rs\"." } },
                    "required": ["path"],
                }),
            },
            ToolSpec {
                name: "search",
                description: "Find lines containing some text (case-insensitive) across the project's files. Returns path:line: text.",
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "query": { "type": "string", "description": "Text to look for, e.g. a type or function name." },
                        "path": { "type": "string", "description": "Optional folder to search in, relative to the project root." },
                    },
                    "required": ["query"],
                }),
            },
        ]
    }

    fn call(&self, name: &str, input: &Value) -> String {
        let result = match name {
            "list_dir" => self.list_dir(input),
            "read_file" => self.read_file(input),
            "search" => self.search(input),
            other => Err(format!("unknown tool {other}")),
        };
        result.unwrap_or_else(|error| format!("Error: {error}"))
    }

    fn describe(&self, name: &str, input: &Value) -> String {
        let arg = |key: &str| input[key].as_str().unwrap_or(".").to_string();
        match name {
            "list_dir" => format!("Listing {}", arg("path")),
            "read_file" => format!("Reading {}", arg("path")),
            "search" => format!("Searching for “{}”", arg("query")),
            other => format!("Running {other}"),
        }
    }
}

/// Files that often hold credentials. The model never sees these.
fn is_secret_file(name: &str) -> bool {
    let name = name.to_lowercase();
    name == ".env"
        || name.starts_with(".env.")
        || name.ends_with(".env")
        || [".pem", ".key", ".p12", ".pfx", ".keystore", ".jks"]
            .iter()
            .any(|ext| name.ends_with(ext))
        || name.starts_with("id_rsa")
        || name.starts_with("id_ed25519")
        || name.starts_with("id_ecdsa")
        || name.starts_with("credentials")
        || [".netrc", ".npmrc", ".pypirc", ".git-credentials"].contains(&name.as_str())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    fn project() -> (tempfile::TempDir, ProjectTools) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join("src/models")).unwrap();
        fs::create_dir_all(root.join("target/debug")).unwrap();
        fs::create_dir_all(root.join(".git")).unwrap();
        fs::write(root.join(".git/config"), "secret-ish").unwrap();
        fs::write(root.join(".gitignore"), "target/\n*.log\n").unwrap();
        fs::write(root.join("src/lib.rs"), "pub mod models;\n").unwrap();
        fs::write(
            root.join("src/models/user.rs"),
            "pub struct User {\n    pub email: String,\n}\n",
        )
        .unwrap();
        fs::write(root.join("target/debug/out.rs"), "struct User;").unwrap();
        fs::write(root.join("debug.log"), "User logged in").unwrap();
        fs::write(root.join(".env"), "API_KEY=sk-123 User").unwrap();
        fs::write(root.join("server.key"), "User").unwrap();
        let tools = ProjectTools::new(root).unwrap();
        (dir, tools)
    }

    fn call(tools: &ProjectTools, name: &str, input: Value) -> String {
        tools.call(name, &input)
    }

    #[test]
    fn lists_without_ignored_or_secret_files() {
        let (_dir, tools) = project();
        let listing = call(&tools, "list_dir", json!({}));
        assert!(listing.contains("src/"), "{listing}");
        assert!(listing.contains(".gitignore"));
        let entries: Vec<&str> = listing.lines().collect();
        for hidden in ["target/", ".git/", "debug.log", ".env", "server.key"] {
            assert!(!entries.contains(&hidden), "{hidden} leaked: {listing}");
        }
        assert_eq!(
            call(&tools, "list_dir", json!({ "path": "src/models" })),
            "src/models/user.rs"
        );
    }

    #[test]
    fn reads_project_files_only() {
        let (_dir, tools) = project();
        assert!(
            call(&tools, "read_file", json!({ "path": "src/models/user.rs" }))
                .contains("pub email")
        );
        for path in [
            ".env",
            "server.key",
            "debug.log",
            "target/debug/out.rs",
            ".git/config",
        ] {
            let result = call(&tools, "read_file", json!({ "path": path }));
            assert!(
                result.starts_with("Error:"),
                "{path} was readable: {result}"
            );
        }
        for path in ["../outside.rs", "/etc/passwd", "src/../../x"] {
            assert!(
                call(&tools, "read_file", json!({ "path": path })).starts_with("Error:"),
                "{path}"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn symlinks_cannot_escape() {
        let (dir, tools) = project();
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("secret.txt"), "nope").unwrap();
        std::os::unix::fs::symlink(
            outside.path().join("secret.txt"),
            dir.path().join("src/link.txt"),
        )
        .unwrap();
        assert!(
            call(&tools, "read_file", json!({ "path": "src/link.txt" }))
                .contains("outside the project")
        );
    }

    #[test]
    fn search_skips_ignored_and_secret_files() {
        let (_dir, tools) = project();
        let found = call(&tools, "search", json!({ "query": "user" }));
        assert!(
            found.contains("src/models/user.rs:1: pub struct User {"),
            "{found}"
        );
        for hidden in ["target", "debug.log", ".env", "server.key"] {
            assert!(!found.contains(hidden), "{hidden} leaked: {found}");
        }
        assert!(call(&tools, "search", json!({ "query": "zzz" })).starts_with("No matches"));
    }

    #[test]
    fn long_files_are_cut() {
        let (dir, tools) = project();
        fs::write(
            dir.path().join("src/big.rs"),
            "x".repeat(MAX_READ_BYTES * 2),
        )
        .unwrap();
        let text = call(&tools, "read_file", json!({ "path": "src/big.rs" }));
        assert!(text.ends_with("bytes]"));
        assert!(text.len() < MAX_READ_BYTES + 100);
    }
}
