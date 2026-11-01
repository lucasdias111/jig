//! Opening the system's terminal for an agent's own sign-in.

use jig_ai::agent::acp::AgentCommandLine;
use jig_process::NoWindow as _;

/// Open the system's terminal running `command`, for an agent's own login.
/// The window stays open when it's done, saying to go back to Jig.
pub fn open_in_terminal(command: &AgentCommandLine) -> anyhow::Result<()> {
    use anyhow::Context as _;
    let quote = |word: &str| format!("'{}'", word.replace('\'', r"'\''"));
    let line = std::iter::once(command.program.to_string_lossy().into_owned())
        .chain(command.args.iter().cloned())
        .map(|word| quote(&word))
        .collect::<Vec<_>>()
        .join(" ");
    let mut script = String::from("#!/bin/sh\n");
    if let Some(path) = &command.path {
        script.push_str(&format!("export PATH={}\n", quote(&path.to_string_lossy())));
    }
    for (key, value) in &command.env {
        script.push_str(&format!("export {key}={}\n", quote(value)));
    }
    script.push_str(&format!(
        "{line}\necho\necho 'Done. Close this window and go back to Jig.'\n"
    ));
    if cfg!(windows) {
        let mut words = vec![
            "/c".to_string(),
            "start".into(),
            String::new(),
            "cmd".into(),
            "/k".into(),
        ];
        words.push(command.program.to_string_lossy().into_owned());
        words.extend(command.args.iter().cloned());
        std::process::Command::new("cmd")
            .no_window()
            .args(words)
            .envs(command.env.iter().map(|(k, v)| (k, v)))
            .spawn()
            .context("Couldn't open a terminal")?;
        return Ok(());
    }
    let file = std::env::temp_dir().join(if cfg!(target_os = "macos") {
        "jig-sign-in.command"
    } else {
        "jig-sign-in.sh"
    });
    std::fs::write(&file, script).context("Couldn't write the sign-in script")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o700))?;
    }
    if cfg!(target_os = "macos") {
        // A .command file opens in the user's terminal app.
        std::process::Command::new("open")
            .arg(&file)
            .spawn()
            .context("Couldn't open a terminal")?;
        return Ok(());
    }
    let terminals: [(&str, &[&str]); 4] = [
        ("x-terminal-emulator", &["-e"]),
        ("gnome-terminal", &["--"]),
        ("konsole", &["-e"]),
        ("xterm", &["-e"]),
    ];
    for (terminal, flags) in terminals {
        if std::process::Command::new(terminal)
            .args(flags)
            .arg(&file)
            .spawn()
            .is_ok()
        {
            return Ok(());
        }
    }
    anyhow::bail!("Couldn't find a terminal. Run this in one: {line}")
}
