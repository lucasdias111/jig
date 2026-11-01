//! Run configurations: the commands a project is run, tested and built
//! with, as IntelliJ has them.

pub use crate::domain::run_configs::{
    DebugProgram, FILE, RunConfig, Source, TEMPLATE, parse_file, shell_word, split_command,
};
pub use crate::store::configs::{detect, from_file, load};

#[cfg(test)]
mod tests;
