//! What git's output means, in plain Rust: the status, branches and
//! failures it reports ([`status`]), and the gutter's change bars
//! ([`hunks`]).

pub mod hunks;
pub mod status;

#[cfg(test)]
mod tests;
