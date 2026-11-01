//! Search as plain Rust: Go to File's list ([`files`]) and searching the
//! text of files ([`text_search`]), which reads them through a trait the
//! store implements. No GPUI and no disk.

pub mod files;
pub mod text_search;
