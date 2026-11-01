//! The debuggers as they are now ([`cache`]), each on disk: installing,
//! uninstalling and whether it can start ([`persist`]), and the programs
//! and tools it needs on this computer ([`programs`]).

pub mod cache;
pub mod persist;
pub mod programs;
