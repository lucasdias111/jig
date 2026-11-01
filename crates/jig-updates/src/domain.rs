//! The rules on updates, in plain Rust: which release is newer, that its
//! build is signed as ours ([`release`]), and whether this copy of Jig can
//! replace itself ([`install`]).

pub mod install;
pub mod release;
