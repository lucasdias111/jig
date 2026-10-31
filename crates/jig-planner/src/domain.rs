//! The planner's rules, in plain Rust: no GPUI and no disk, so they're
//! tested without a window or a folder. What they need kept or read comes
//! through a trait the store implements ([`tasks::TaskStore`],
//! [`symbols::ProjectFiles`]).

pub mod links;
pub mod mention;
pub mod symbols;
pub mod tasks;
