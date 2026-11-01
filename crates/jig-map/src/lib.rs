//! The Map: one canvas you zoom out to from anywhere in the code, showing
//! how the project's functions relate, then its modules and files. It's
//! for understanding and deciding, and never changes code.
//!
//! In layers: `domain` is what the Map knows and decides (the index built
//! from each file's outline, the levels, domains, what the agent draws
//! and what the user sketches), with no IO; `store` reads the project for
//! the index and keeps `.jig/map/`; `ui` draws the canvas, zooms between
//! the levels and takes the gestures, and serves the Map's tools to the
//! agent.

mod domain;
mod store;
mod ui;

// The Map's contract: what the rest of Jig uses of it. Everything else in
// here is the Map's own, and private.
pub use domain::domains::actions::{Action, only_reads};
pub use domain::index::{FnId, Project, Target};
pub use domain::lens::{Call, Drawing, Kind as DrawingKind, call as drawing_call};
pub use domain::plan::Focus;
pub use domain::site::Show as SiteShows;
pub use domain::sketch::{
    DIR as SKETCH_DIR, Match as SketchMatch, ShapeKind, ShapeRef, link as sketch_link,
    of_link as sketch_of_link,
};
pub use store::cache::root_of;
#[cfg(any(test, feature = "testing"))]
pub use store::fixture::fixture as fixture_project;
pub use ui::mcp::{project_from_args, serve as serve_drawing_tools};
pub use ui::view::lens::Source as DrawingSource;
pub use ui::view::ruler::ruler;
pub use ui::view::sketch::Tool as SketchTool;
pub use ui::view::{Level, MapEvent, MapView, ProposalChoice, key_bindings};
