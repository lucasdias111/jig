//! The servers themselves: each one's process and connection
//! ([`client`]), which server and root a file gets and how it's started
//! ([`servers`]), installing them ([`install`]), and the names problems
//! are kept under ([`diagnostics`]).

pub mod client;
pub mod diagnostics;
pub mod install;
pub mod servers;
