//! The file behind the editor: where it lives on disk and what was last saved.

use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};

use crate::languages::language_for;
use crate::settings::LanguageSettings;

#[derive(Debug, Default)]
pub struct Document {
    /// `None` until the buffer is first saved.
    pub path: Option<PathBuf>,
    /// The text as last read from or written to disk.
    pub saved_text: String,
}

impl Document {
    pub fn open(path: &Path) -> Result<Self> {
        let saved_text =
            fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        Ok(Self {
            path: Some(path.to_path_buf()),
            saved_text,
        })
    }

    pub fn save(&mut self, path: &Path, text: &str) -> Result<()> {
        atomic_write(path, text)?;
        self.path = Some(path.to_path_buf());
        self.saved_text = text.to_string();
        Ok(())
    }

    pub fn is_dirty(&self, text: &str) -> bool {
        self.saved_text != text
    }

    pub fn title(&self) -> String {
        self.path
            .as_deref()
            .and_then(Path::file_name)
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Untitled".into())
    }

    /// The language to highlight the file with; see [`language_for`].
    pub fn language(&self, settings: &LanguageSettings) -> &'static str {
        self.path
            .as_deref()
            .map_or("text", |path| language_for(path, settings))
    }

    /// The file's line endings as last saved: `"CRLF"` if its first line
    /// ends that way, otherwise `"LF"`.
    pub fn line_ending(&self) -> &'static str {
        match self.saved_text.find('\n') {
            Some(ix) if self.saved_text[..ix].ends_with('\r') => "CRLF",
            _ => "LF",
        }
    }

    /// Files are only ever read as UTF-8; a byte order mark is kept.
    pub fn encoding(&self) -> &'static str {
        if self.saved_text.starts_with('\u{feff}') {
            "UTF-8 with BOM"
        } else {
            "UTF-8"
        }
    }
}

/// Write `text` to a temporary file next to `path`, then rename it over
/// `path`, so a crash mid-save never leaves a half-written file. Keeps the
/// original file's permissions.
pub fn atomic_write(path: &Path, text: &str) -> Result<()> {
    let dir = path
        .parent()
        .filter(|dir| !dir.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let name = path
        .file_name()
        .context("saving to a path without a file name")?;
    let tmp = dir.join(format!(".{}.jig-tmp", name.to_string_lossy()));

    let result = (|| {
        let mut file =
            fs::File::create(&tmp).with_context(|| format!("creating {}", tmp.display()))?;
        file.write_all(text.as_bytes())?;
        file.sync_all()?;
        if let Ok(metadata) = fs::metadata(path) {
            fs::set_permissions(&tmp, metadata.permissions())?;
        }
        fs::rename(&tmp, path).with_context(|| format!("replacing {}", path.display()))
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn save_round_trip_and_dirty_tracking() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.rs");
        fs::write(&path, "fn a() {}\n").unwrap();

        let mut doc = Document::open(&path).unwrap();
        assert_eq!(doc.title(), "a.rs");
        assert_eq!(doc.language(&LanguageSettings::default()), "rust");
        assert!(!doc.is_dirty("fn a() {}\n"));
        assert!(doc.is_dirty("fn b() {}\n"));

        doc.save(&path, "fn b() {}\n").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "fn b() {}\n");
        assert!(!doc.is_dirty("fn b() {}\n"));
        // No temp file left behind.
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn save_keeps_permissions() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("run.sh");
        fs::write(&path, "echo hi\n").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();

        atomic_write(&path, "echo bye\n").unwrap();
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o755
        );
    }

    #[test]
    fn untitled_document() {
        let doc = Document::default();
        assert_eq!(doc.title(), "Untitled");
        assert_eq!(doc.language(&LanguageSettings::default()), "text");
        assert_eq!(doc.line_ending(), "LF");
        assert_eq!(doc.encoding(), "UTF-8");
    }

    #[test]
    fn line_endings_and_bom() {
        let doc = |text: &str| Document {
            path: None,
            saved_text: text.into(),
        };
        assert_eq!(doc("a\r\nb\r\n").line_ending(), "CRLF");
        assert_eq!(doc("a\nb\r\n").line_ending(), "LF");
        assert_eq!(doc("\u{feff}a\n").encoding(), "UTF-8 with BOM");
    }
}
