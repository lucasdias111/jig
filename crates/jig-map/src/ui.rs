//! The Map's canvas: drawing the levels, zooming between them and taking
//! the gestures.

/// The Map's tools served to the agent over stdio (`Jig --map-mcp`): the
/// agent's way into the Map, as the canvas is the user's, so it sits with
/// the views, the one layer that reaches both the domain and the store.
pub mod mcp;
pub mod view;
