//! Git, through the `git` command line rather than a library, so the user's
//! own config, hooks, credential helpers and SSH agent all apply. Every call
//! blocks; the workspace runs them in the background.
//!
//! This file is the crate's facade: it only says what the rest of Jig may
//! use. The work is in its layers (`docs/layers.md`):
//!
//! - [`ui`]: the Git view and the branch picker.
//! - [`domain`]: reading git's status and branches, and the change bars'
//!   hunks, in plain Rust.
//! - [`store`]: the repository, through the command line.

mod domain;
mod store;
mod ui;

pub use domain::hunks::{Hunk, hunks};
pub use domain::status::{Branch, Change, FileStatus, MAX_NEW_FOLDER_FILES, Status};
pub use store::persist::Repo;
pub use ui::branch_picker::{
    BranchPicker, BranchPickerEvent, Mode as BranchPickerMode, WIDTH as BRANCH_PICKER_WIDTH,
};
pub use ui::panel::{BRANCH_ICON, GitPanel, GitPanelEvent};
