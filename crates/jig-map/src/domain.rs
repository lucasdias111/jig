//! What the Map knows and decides, in plain Rust: the project's outline
//! and calls (`index`, `outline`), the levels built from them (`plan`,
//! `site`, `domains`, `layout`, `files`), what the agent draws (`lens`)
//! and what the user sketches (`sketch`). Nothing here draws or touches
//! the disk.

pub mod debug;
pub mod domains;
pub mod files;
pub mod index;
pub mod layout;
pub mod lens;
pub mod outline;
pub mod plan;
pub mod site;
pub mod sketch;
