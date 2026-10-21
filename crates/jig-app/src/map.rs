//! The Map: one canvas you zoom out to from anywhere in the code, showing
//! how the project's functions relate, then its modules and files. It's
//! for understanding and deciding, and never changes code.
//!
//! `outline` reads a file, `index` the whole project; `plan` builds the
//! neighbourhood of a function or a module (`files` the links of a file
//! the index can't read), `site` the whole project nested; `layout`
//! places Plan's boxes; `lens` is what the agent draws on it; `view` draws
//! the canvas, zooms between the levels and takes the gestures.

pub mod debug;
pub mod domains;
pub mod files;
pub mod index;
pub mod layout;
pub mod lens;
pub mod outline;
pub mod plan;
pub mod site;
pub mod view;
