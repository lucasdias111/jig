//! Where the table lives: read once at startup ([`session`]), and the
//! user's file and the servers' programs on disk ([`persist`]).

pub mod persist;
pub mod session;
