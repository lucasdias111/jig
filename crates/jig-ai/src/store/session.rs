//! What lasts as long as this process: the executable agents start Jig's
//! tools with, and the id Jig's requests carry.

use std::path::PathBuf;
use std::sync::OnceLock;

static COMMAND: OnceLock<PathBuf> = OnceLock::new();

/// The executable agents start the tool with, normally Jig itself. Until
/// it's set, as in tests, agents get no task tool.
pub fn set_command(executable: PathBuf) {
    let _ = COMMAND.set(executable);
}

/// The executable agents start Jig's tools with, once it's set.
pub fn command() -> Option<&'static PathBuf> {
    COMMAND.get()
}

/// One ID per Jig launch, stable across that run's requests.
pub fn session_id() -> &'static str {
    static ID: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    ID.get_or_init(|| {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_nanos());
        format!("jig-{:x}-{nanos:x}", std::process::id())
    })
}
