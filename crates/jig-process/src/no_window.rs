//! Keeping console programs from opening windows of their own.

use std::process::Command;

/// Keeps a console program (git, a language server, a shell) from opening a
/// console window of its own on Windows, as it would when started by Jig,
/// which has none. Elsewhere it changes nothing.
pub trait NoWindow {
    fn no_window(&mut self) -> &mut Self;
}

impl NoWindow for Command {
    fn no_window(&mut self) -> &mut Self {
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt as _;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            self.creation_flags(CREATE_NO_WINDOW);
        }
        self
    }
}
