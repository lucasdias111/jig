//! The Map: one canvas you zoom out to from anywhere in the code, showing
//! how the project's functions relate, then its modules and files. It's
//! for understanding and deciding, and never changes code.
//!
//! `outline` reads a file, `index` the whole project; `plan` builds the
//! neighbourhood of a function or a module (`files` the links of a file
//! the index can't read), `site` the whole project nested; `layout`
//! places Plan's boxes; `lens` is what the agent draws on it, `sketch`
//! what the user does; `view` draws
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
pub mod sketch;
pub mod view;

// The Map's contract: what the rest of Jig uses of it. Everything else in
// here is the Map's own (`architecture.toml`).
pub use domains::actions::{Action, only_reads};
pub use index::{FnId, Project, Target, root_of};
pub use lens::mcp::{project_from_args, serve as serve_drawing_tools};
pub use lens::{Call, Drawing, Kind as DrawingKind, call as drawing_call};
pub use sketch::{link as sketch_link, of_link as sketch_of_link};
pub use view::lens::Source as DrawingSource;
pub use view::ruler::ruler;
pub use view::{Level, MapEvent, MapView, ProposalChoice, key_bindings};
