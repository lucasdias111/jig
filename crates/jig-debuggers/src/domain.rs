//! The debuggers as data, in plain Rust: parsing and merging the built-in
//! and the user's files, and filling in `${variables}`. The tools those
//! variables name are found through [`vars::Tools`], which the store
//! implements.

pub mod debuggers;
pub mod vars;
