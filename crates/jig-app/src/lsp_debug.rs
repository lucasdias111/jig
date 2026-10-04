//! Debuggers that live inside a language server, as Java's does in jdtls:
//! wait for the server to load the project, work out what the launch needs
//! (for jdtls, the main class and classpath), then ask it to start a debug
//! session, which it answers with the port its adapter listens on.
//!
//! All of it blocks on the server, so it runs on a thread of its own.

use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::lsp::Client;

/// How long a server gets to load the project; jdtls importing a large
/// Maven project for the first time can take minutes.
const READY_TIMEOUT: Duration = Duration::from_secs(300);
/// How long one command gets to answer.
const COMMAND_TIMEOUT: Duration = Duration::from_secs(60);

/// A started debug session: its adapter's port, and values for the launch
/// template.
pub struct Started {
    pub port: u16,
    pub values: Vec<(String, Value)>,
}

/// Start a debug session with `start_command` on `server`'s `client` for
/// the project at `root`. `prefer` are files whose program to debug when
/// the project has several, most wanted first.
pub fn start(
    client: &Client,
    server: &str,
    start_command: &str,
    root: &Path,
    prefer: &[PathBuf],
) -> Result<Started, String> {
    let values = if server == "jdtls" {
        wait_until_ready(client, server)?;
        java_values(client, root, prefer)?
    } else {
        Vec::new()
    };
    let port = call(client, start_command, json!([]))?
        .as_u64()
        .and_then(|port| u16::try_from(port).ok())
        .ok_or_else(|| format!("{server} didn't start a debug session."))?;
    Ok(Started { port, values })
}

fn wait_until_ready(client: &Client, server: &str) -> Result<(), String> {
    let started = Instant::now();
    while !client.is_service_ready() {
        if client.is_dead() {
            return Err(format!("{server} exited."));
        }
        if started.elapsed() > READY_TIMEOUT {
            return Err(format!("{server} didn't finish loading the project."));
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    Ok(())
}

/// `${java_main_class}`, `${java_project}`, `${java_class_paths}` and
/// `${java_module_paths}`, from jdtls.
fn java_values(
    client: &Client,
    root: &Path,
    prefer: &[PathBuf],
) -> Result<Vec<(String, Value)>, String> {
    let mains = call(client, "vscode.java.resolveMainClass", json!([root]))?;
    let mains = mains.as_array().cloned().unwrap_or_default();
    let canonical = |path: &Path| path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let wanted: Vec<PathBuf> = prefer.iter().map(|path| canonical(path)).collect();
    let main = wanted
        .iter()
        .find_map(|want| {
            mains.iter().find(|main| {
                main["filePath"]
                    .as_str()
                    .is_some_and(|file| canonical(Path::new(file)) == *want)
            })
        })
        .or_else(|| mains.first())
        .ok_or("No class in the project has a main method.")?;
    let class = main["mainClass"].as_str().unwrap_or_default().to_string();
    let project = main["projectName"].as_str().unwrap_or_default().to_string();
    let paths = call(
        client,
        "vscode.java.resolveClasspath",
        json!([class, project]),
    )?;
    Ok(vec![
        ("java_main_class".into(), json!(class)),
        ("java_project".into(), json!(project)),
        ("java_module_paths".into(), paths[0].clone()),
        ("java_class_paths".into(), paths[1].clone()),
    ])
}

/// Run `workspace/executeCommand` and wait for its result.
fn call(client: &Client, command: &str, arguments: Value) -> Result<Value, String> {
    let response = client.request(
        "workspace/executeCommand",
        json!({"command": command, "arguments": arguments}),
    );
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(futures::executor::block_on(response));
    });
    match rx.recv_timeout(COMMAND_TIMEOUT) {
        Ok(Ok(Ok(result))) => Ok(result),
        Ok(Ok(Err(error))) => Err(format!("{command} failed: {error}")),
        Ok(Err(_)) | Err(mpsc::RecvTimeoutError::Disconnected) => {
            Err(format!("The language server stopped answering {command}."))
        }
        Err(mpsc::RecvTimeoutError::Timeout) => Err(format!("{command} took too long.")),
    }
}
