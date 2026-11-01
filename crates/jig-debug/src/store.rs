//! The adapters themselves: connections and the processes behind them
//! ([`client`]), asking a language server to start one ([`lsp_debug`]),
//! and planning against the files on disk ([`programs`]).

pub mod client;
pub mod lsp_debug;
pub mod programs;
