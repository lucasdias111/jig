//! The debuggers as they are now, read again when the user's file
//! changes, and which `check`s passed.

use std::collections::HashMap;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use jig_process::NoWindow as _;

use super::persist::user_path;
use super::programs::resolve_program;
use crate::domain::debuggers::{Debugger, Registry, merge};
use crate::domain::vars::Vars;

/// Which `check`s passed, so Settings doesn't run them on every repaint.
static CHECKS: Mutex<Option<HashMap<String, bool>>> = Mutex::new(None);

/// Forgets which `check`s passed, after something was installed or
/// removed.
pub(crate) fn forget_checks() {
    *CHECKS.lock().unwrap() = None;
}

/// The debuggers as they are now; the user's file is read again when it
/// has changed.
pub fn registry() -> Arc<Registry> {
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
    forget_checks();
    *cache = Some((modified, registry.clone()));
    registry
}

/// The built-ins with the user's file at `user`, if it reads.
pub fn load(user: Option<&Path>) -> Registry {
    let text = user.and_then(|path| std::fs::read_to_string(path).ok());
    merge(text.as_deref())
}

impl Debugger {
    /// Whether its `check` command passes, or it has none. Run once, then
    /// remembered.
    pub(crate) fn passes_check(&self, vars: &Vars) -> bool {
        let Some(check) = &self.check else {
            return true;
        };
        let mut checks = CHECKS.lock().unwrap();
        let checks = checks.get_or_insert_with(HashMap::new);
        if let Some(passed) = checks.get(&self.key) {
            return *passed;
        }
        let passed = (|| {
            let words = check
                .iter()
                .map(|word| vars.expand_str(word))
                .collect::<Result<Vec<_>, _>>()
                .ok()?;
            let env = self
                .adapter_env
                .iter()
                .map(|(key, value)| Some((key.clone(), vars.expand_str(value).ok()?)))
                .collect::<Option<Vec<_>>>()?;
            let status = Command::new(resolve_program(&words[0]).ok()?)
                .no_window()
                .args(&words[1..])
                .envs(env)
                .env("PATH", jig_tools::search_path())
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .ok()?;
            Some(status.success())
        })()
        .unwrap_or(false);
        checks.insert(self.key.clone(), passed);
        passed
    }
}
