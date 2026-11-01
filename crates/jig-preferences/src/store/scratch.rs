//! The folders the Settings snapshots write to: the pictures' folder, and a
//! scratch config folder so the pictures never show the user's own settings.

use std::fs;
use std::path::{Path, PathBuf};

/// Make `dir`, for the pictures.
pub fn make_folder(dir: &Path) {
    fs::create_dir_all(dir).expect("making the snapshots folder");
}

/// A fresh config folder of this process's own.
pub fn scratch_config() -> PathBuf {
    let config = std::env::temp_dir().join(format!("jig-snapshots-{}", std::process::id()));
    fs::create_dir_all(&config).expect("making a scratch config folder");
    config
}

/// Take the scratch config folder away again.
pub fn remove_folder(dir: &Path) {
    fs::remove_dir_all(dir).ok();
}
