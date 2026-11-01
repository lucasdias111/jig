//! The formatters as they are now, read again when the user's file
//! changes.

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use super::persist::user_path;
use crate::domain::formatters::{Registry, merge};

/// The formatters as they are now; the user's file is read again when it
/// has changed.
pub fn registry() -> Arc<Registry> {
    #[cfg(any(test, feature = "testing"))]
    if let Some(registry) = TEST_REGISTRY.with(|test| test.borrow().clone()) {
        return registry;
    }
    type Cached = (Option<SystemTime>, Arc<Registry>);
    static CACHE: Mutex<Option<Cached>> = Mutex::new(None);
    let path = user_path();
    let modified = path
        .as_deref()
        .and_then(|path| std::fs::metadata(path).ok())
        .and_then(|meta| meta.modified().ok());
    let mut cache = CACHE.lock().unwrap();
    if let Some((stamp, registry)) = cache.as_ref()
        && *stamp == modified
    {
        return registry.clone();
    }
    let registry = Arc::new(load(path.as_deref()));
    *cache = Some((modified, registry.clone()));
    registry
}

/// The built-ins with the user's file at `user`, if it reads.
pub fn load(user: Option<&Path>) -> Registry {
    let text = user.and_then(|path| std::fs::read_to_string(path).ok());
    merge(text.as_deref())
}

#[cfg(any(test, feature = "testing"))]
thread_local! {
    static TEST_REGISTRY: std::cell::RefCell<Option<Arc<Registry>>> =
        const { std::cell::RefCell::new(None) };
}

/// Use only the formatters in `toml` on this thread, for tests: none of
/// the built-ins, which may well be installed.
#[cfg(any(test, feature = "testing"))]
pub fn use_for_tests(toml: &str) {
    let registry = Registry {
        formatters: crate::domain::formatters::parse(toml)
            .unwrap()
            .into_iter()
            .map(Arc::new)
            .collect(),
        error: None,
    };
    TEST_REGISTRY.with(|test| *test.borrow_mut() = Some(Arc::new(registry)));
}
