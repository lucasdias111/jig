//! Running a configuration and reading its output, colours and file
//! references kept.

pub use crate::domain::output::{
    Color, Link, OutputLine, RunEvent, Stream, Style, find_links, strip_ansi,
};
pub use crate::store::process::{Process, parse_line, shell_command};

#[cfg(test)]
mod tests;
