//! The tree's state and how it's kept: the folders' listings, read from
//! disk and kept until refreshed ([`cache`]), and the changes made to the
//! files themselves ([`persist`]).

pub mod cache;
pub mod persist;

#[cfg(test)]
mod tests;
