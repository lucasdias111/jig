//! The grammars, registered with GPUI Kit's highlighter.

pub mod gdscript;
pub mod grammars;

/// Registers the grammars GPUI Kit has no feature for, before any file opens.
pub fn register_grammars() {
    gdscript::register();
    grammars::register();
}
