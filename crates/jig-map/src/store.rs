//! What the Map keeps: the index, a cache of the project on disk
//! (`cache`), and what's saved in the project's `.jig/map/` (`persist`).

pub mod cache;
/// A project to read in tests, here and in the app's.
#[cfg(any(test, feature = "testing"))]
pub mod fixture;
pub mod persist;
