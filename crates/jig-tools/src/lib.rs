//! Installing the tools Jig runs, as Neovim's Mason does: language servers,
//! debuggers and formatters each describe an `install` shell command, which
//! runs here into a folder of Jig's, where Jig then finds what it put. Tools
//! installed some other way (rustup, Homebrew, Xcode) are found on the PATH
//! and left alone.
//!
//! This file is the crate's facade. The work is in its layers:
//!
//! - [`domain`]: a tool's help text and the paths it goes by.
//! - [`store`]: the `PATH`, Jig's folders, and running installers.

mod domain;
mod store;

pub use domain::tools::{Help, bin_dirs, expand_platform, home_relative};
pub use store::cache::{find_program, search_path};
pub use store::persist::{
    data_dir, debuggers_dir, dir, executable_in, install_own_folder, run_installer, servers_dir,
    user_file,
};
