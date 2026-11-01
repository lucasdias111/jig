//! Guessing where a name is declared, for Cmd+click when no language server
//! answers (see [`crate::lsp`]), as Vim's `gd` does.

pub use crate::domain::definitions::{
    Target, declaration_in_file, is_member_access, name_at, next_occurrence, resolve,
};
