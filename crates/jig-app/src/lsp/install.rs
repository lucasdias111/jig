//! Installing language servers from Settings, as debuggers are: each
//! server's `install` command runs in a folder of its own in Jig's servers
//! folder, where Jig then finds it, and Uninstall moves that folder to the
//! Trash. Servers installed some other way (rustup, Homebrew, Xcode) are
//! found on the PATH and left alone.

use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};

use super::Server;

/// Where Jig keeps servers it installs, beside its debuggers and
/// formatters. `JIG_SERVERS_DIR` says otherwise.
pub fn servers_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("JIG_SERVERS_DIR") {
        return Some(PathBuf::from(dir));
    }
    crate::debuggers::debuggers_dir()?
        .parent()
        .map(|dir| dir.join("servers"))
}

/// `server`'s own folder in the servers folder, whether or not it's there.
pub fn server_dir(server: &Server) -> Option<PathBuf> {
    servers_dir().map(|dir| dir.join(&server.name))
}

/// Whether Jig installed `server`, so Uninstall has something to remove.
pub fn installed_by_jig(server: &Server) -> bool {
    server_dir(server).is_some_and(|dir| dir.is_dir())
}

/// What Settings says about `server`: the program it runs, or how to get
/// it, and whether it's ready.
pub fn status(server: &Server) -> (String, bool) {
    if server.commands.is_empty() {
        return ("Turned off in language-servers.toml.".into(), true);
    }
    match super::program_for(server) {
        Some((program, _)) => (
            format!("Using {}", crate::tools::home_relative(&program)),
            true,
        ),
        None if server.installable() => (
            format!("{} isn't installed. Jig can install it.", server.name),
            false,
        ),
        None => (
            format!("{} isn't installed. {}", server.name, server.help()),
            false,
        ),
    }
}

/// Run `server`'s install command. Blocks, so run it off the main thread.
pub fn install(server: &Server) -> Result<()> {
    let dir = server_dir(server).context("No home folder to install into")?;
    install_into(server, &dir)?;
    if super::program_for(server).is_none() {
        bail!(
            "The installer finished, but Jig still can't find {}.",
            server.name
        );
    }
    Ok(())
}

fn install_into(server: &Server, dir: &Path) -> Result<()> {
    let script = server.install.as_deref().context("Nothing to install")?;
    crate::tools::install_own_folder(script, dir)
}

/// Move the folder Jig installed `server` into to the Trash.
pub fn uninstall(server: &Server) -> Result<()> {
    let dir = server_dir(server).filter(|dir| dir.is_dir());
    let Some(dir) = dir else {
        bail!(
            "Jig didn't install {}, so there's nothing to remove.",
            server.name
        );
    };
    trash::delete(&dir).context("Couldn't move it to the Trash")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn server(install: &str) -> Server {
        toml::from_str(&format!(
            "name = \"fake-ls\"\nlanguages = [\"rust\"]\ncommands = [[\"fake-ls\"]]\ninstall = '{install}'\n"
        ))
        .unwrap()
    }

    #[test]
    fn status_says_how_to_get_a_missing_server() {
        let installable = server("true");
        let (text, ready) = status(&installable);
        assert!(!ready);
        assert_eq!(text, "fake-ls isn't installed. Jig can install it.");
        let mut manual = server("true");
        manual.install = None;
        let (text, _) = status(&manual);
        assert_eq!(
            text,
            "fake-ls isn't installed. Install fake-ls and put it on the PATH."
        );
    }

    #[test]
    fn install_runs_the_command_in_the_server_s_folder() {
        let dir = tempfile::tempdir().unwrap();
        let own = dir.path().join("fake-ls");
        install_into(
            &server(r#"mkdir -p "${dir}/bin" && touch "${dir}/bin/made""#),
            &own,
        )
        .unwrap();
        assert!(own.join("bin/made").is_file());
    }

    #[test]
    fn a_failed_install_says_why_and_leaves_no_folder() {
        let dir = tempfile::tempdir().unwrap();
        let own = dir.path().join("fake-ls");
        let error = install_into(&server("echo no npm >&2; exit 3"), &own)
            .unwrap_err()
            .to_string();
        assert!(error.contains("no npm"), "{error}");
        assert!(!own.exists());
    }

    /// Installs YAML's server with npm into `JIG_SERVERS_DIR` and finds it
    /// there: `JIG_SERVERS_DIR=<scratch> cargo test -p jig-app
    /// server_install_live -- --ignored`. Deletes it after rather than
    /// using the Trash, which asks the Finder.
    #[test]
    #[ignore]
    fn server_install_live() {
        assert!(
            std::env::var_os("JIG_SERVERS_DIR").is_some(),
            "set JIG_SERVERS_DIR"
        );
        let server = crate::lsp::server_named("yaml-language-server").unwrap();
        install(server).unwrap();
        assert!(installed_by_jig(server));
        let (status, ready) = status(server);
        assert!(ready, "{status}");
        assert!(
            status.contains("node_modules/.bin/yaml-language-server"),
            "{status}"
        );
        std::fs::remove_dir_all(server_dir(server).unwrap()).unwrap();
        assert!(!installed_by_jig(server));
    }
}
