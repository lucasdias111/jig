//! A debugger on disk: the user's file, whether it can start, installing
//! and uninstalling it, and the plugins it puts in a language server.

use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};
use jig_tools::debuggers_dir;
use serde_json::json;

use super::cache::{forget_checks, registry};
use crate::domain::debuggers::Debugger;
use crate::domain::vars::Vars;

/// The user's `debuggers.toml`; `None` in tests.
pub fn user_path() -> Option<PathBuf> {
    if cfg!(any(test, feature = "testing")) {
        return None;
    }
    std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config/jig/debuggers.toml"))
}

/// The plugins debuggers ask language server `server` to load, that are
/// installed.
pub fn lsp_bundles(server: &str) -> Vec<PathBuf> {
    let vars = Vars::new(&std::env::temp_dir());
    registry()
        .debuggers
        .iter()
        .filter(|d| d.lsp.as_deref() == Some(server))
        .flat_map(|d| d.lsp_bundles.iter())
        .filter_map(|pattern| vars.expand_str(pattern).ok())
        .flat_map(|pattern| glob(&pattern))
        .collect()
}

/// The files matching `pattern`, whose last part may have one `*`.
pub(crate) fn glob(pattern: &str) -> Vec<PathBuf> {
    let path = PathBuf::from(pattern);
    let (Some(dir), Some(name)) = (path.parent(), path.file_name().and_then(|n| n.to_str())) else {
        return Vec::new();
    };
    let Some((before, after)) = name.split_once('*') else {
        return path.is_file().then_some(path.clone()).into_iter().collect();
    };
    let mut found: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .filter_map(|entry| entry.ok())
        .filter(|entry| {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            name.len() >= before.len() + after.len()
                && name.starts_with(before)
                && name.ends_with(after)
        })
        .map(|entry| entry.path())
        .collect();
    found.sort();
    found
}

/// What's missing for a debugger that lives in a language server.
fn lsp_missing(debugger: &Debugger, server: &str) -> Option<String> {
    if !jig_languages::is_installed(server) {
        return Some(format!("{server} isn't installed."));
    }
    let vars = Vars::new(&std::env::temp_dir());
    let missing_plugin = debugger.lsp_bundles.iter().any(|pattern| {
        vars.expand_str(pattern)
            .map_or(true, |pattern| glob(&pattern).is_empty())
    });
    missing_plugin.then(|| {
        format!(
            "{}'s debugger plugin for {server} isn't installed.",
            debugger.name
        )
    })
}

/// Why `debugger` can't start, or `None` when it can.
pub fn missing(debugger: &Debugger, cwd: &Path) -> Option<String> {
    if let Some((server, _)) = debugger.lsp() {
        return lsp_missing(debugger, server);
    }
    let mut vars = Vars::new(cwd);
    vars.set("port", json!("0"));
    match debugger.adapter_command(&vars) {
        Err(what) => Some(what),
        Ok(_) if !debugger.passes_check(&vars) => {
            Some(format!("{}'s debugger isn't installed.", debugger.name))
        }
        Ok(_) => None,
    }
}

/// What Settings says about `debugger`: where its adapter is, or what's
/// missing, and whether it's ready.
pub fn status(debugger: &Debugger) -> (String, bool) {
    let cwd = std::env::temp_dir();
    if let Some(what) = missing(debugger, &cwd) {
        let how = if debugger.installable() {
            "Jig can install it.".to_string()
        } else {
            debugger.help()
        };
        return (format!("{what} {how}"), false);
    }
    if let Some((server, _)) = debugger.lsp() {
        let plugin = lsp_bundles(server)
            .into_iter()
            .next()
            .map(|path| jig_tools::home_relative(&path))
            .unwrap_or_default();
        return (format!("Using {server} with {plugin}"), true);
    }
    let mut vars = Vars::new(&cwd);
    vars.set("port", json!("0"));
    let shown = debugger
        .adapter_command(&vars)
        .map(|adapter| jig_tools::home_relative(&adapter.shown))
        .unwrap_or_default();
    (format!("Using {shown}"), true)
}

/// Run `debugger`'s install command. Blocks, so run it off the main thread.
pub fn install(debugger: &Debugger) -> Result<()> {
    let dir = debuggers_dir().context("No home folder to install into")?;
    install_into(debugger, &dir)
}

pub(crate) fn install_into(debugger: &Debugger, dir: &Path) -> Result<()> {
    let script = debugger.install.as_deref().context("Nothing to install")?;
    let mut vars = Vars::new(dir);
    vars.set("debuggers", json!(dir));
    let script = jig_tools::expand_platform(script);
    let script = vars.expand_str(&script).map_err(anyhow::Error::msg)?;
    let result = jig_tools::run_installer(&script, dir);
    forget_checks();
    result
}

/// What Jig installed of `debugger` that is still there: the `uninstall`
/// paths that exist in the debuggers folder. Paths reaching outside it
/// are ignored, so Uninstall can't remove anything else.
pub fn installed_files(debugger: &Debugger) -> Vec<PathBuf> {
    debuggers_dir()
        .map(|dir| installed_in(debugger, &dir))
        .unwrap_or_default()
}

pub(crate) fn installed_in(debugger: &Debugger, dir: &Path) -> Vec<PathBuf> {
    debugger
        .uninstall
        .iter()
        .map(Path::new)
        .filter(|path| {
            path.components()
                .all(|part| matches!(part, std::path::Component::Normal(_)))
        })
        .map(|path| dir.join(path))
        .filter(|path| path.symlink_metadata().is_ok())
        .collect()
}

/// Move what Jig installed of `debugger` to the Trash.
pub fn uninstall(debugger: &Debugger) -> Result<()> {
    let files = installed_files(debugger);
    if files.is_empty() {
        bail!(
            "Jig didn't install {}, so there's nothing to remove.",
            debugger.name
        );
    }
    let result = trash::delete_all(&files).context("Couldn't move it to the Trash");
    forget_checks();
    result
}
