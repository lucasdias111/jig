//! Small shared pieces with no feature of their own: the theme, finding a
//! project's root, .editorconfig, the preview diff, line edits, indentation,
//! brackets and the field box.
//!
//! This file is the crate's facade: it only says what the rest of Jig may
//! use, by the module paths it always had. The work is in its layers
//! (`docs/layers.md`):
//!
//! - [`ui`]: the theme, the field box, and indentation as the editor takes it.
//! - [`domain`]: brackets, the diff, line edits, indentation and
//!   `.editorconfig` rules, in plain Rust.
//! - [`store`]: what's read from disk: project roots and `.editorconfig`s.

mod domain;
mod store;
mod ui;

pub use domain::{brackets, diff, lines};
pub use store::project;
pub use ui::{field_box, theme};

/// A file's `.editorconfig` properties: read by [`store`], with the tab
/// size the editor takes from [`ui`].
pub mod editorconfig {
    pub use crate::domain::editorconfig::{EditorConfig, LineEnding};
}

/// Which indentation a file already uses, so Tab keeps to it and the status
/// bar can say what it is.
pub mod indentation {
    pub use crate::domain::indentation::active_guide;
    pub use crate::ui::indentation::{DEFAULT, detect, label};
}
