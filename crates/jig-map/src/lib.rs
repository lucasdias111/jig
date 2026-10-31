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

mod debug;
mod domains;
mod files;
mod index;
mod layout;
mod lens;
mod outline;
mod plan;
mod site;
mod sketch;
mod view;

// The Map's contract: what the rest of Jig uses of it. Everything else in
// here is the Map's own, and private.
pub use domains::actions::{Action, only_reads};
#[cfg(any(test, feature = "testing"))]
pub use index::fixture::fixture as fixture_project;
pub use index::{FnId, Project, Target, root_of};
pub use lens::mcp::{project_from_args, serve as serve_drawing_tools};
pub use lens::{Call, Drawing, Kind as DrawingKind, call as drawing_call};
pub use plan::Focus;
pub use site::Show as SiteShows;
pub use sketch::{
    DIR as SKETCH_DIR, Match as SketchMatch, ShapeKind, ShapeRef, link as sketch_link,
    of_link as sketch_of_link,
};
pub use view::lens::Source as DrawingSource;
pub use view::ruler::ruler;
pub use view::sketch::Tool as SketchTool;
pub use view::{Level, MapEvent, MapView, ProposalChoice, key_bindings};
