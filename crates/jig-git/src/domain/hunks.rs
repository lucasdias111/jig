//! The gutter's change bars: how the buffer's lines differ from the
//! committed file.

use std::ops::Range;
use std::time::Duration;

/// How a run of lines in the buffer differs from the committed file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hunk {
    /// The buffer's lines, 0-based; empty for a deletion, which sits above
    /// `lines.start`.
    pub lines: Range<usize>,
    /// The committed lines these replace, each with its newline.
    pub old_text: String,
}

impl Hunk {
    pub fn is_deletion(&self) -> bool {
        self.lines.is_empty()
    }

    pub fn is_addition(&self) -> bool {
        self.old_text.is_empty()
    }
}

/// The runs of lines `current` changes in `base`. A diff that takes too long
/// settles for a coarser answer rather than holding up typing.
pub fn hunks(base: &str, current: &str) -> Vec<Hunk> {
    // A checkout can write CRLF where the editor holds LF.
    let base = if current.contains('\r') {
        std::borrow::Cow::Borrowed(base)
    } else {
        std::borrow::Cow::Owned(base.replace("\r\n", "\n"))
    };
    let diff = similar::TextDiff::configure()
        .algorithm(similar::Algorithm::Myers)
        .timeout(Duration::from_millis(50))
        .diff_lines(base.as_ref(), current);
    let old_lines: Vec<&str> = base.split_inclusive('\n').collect();
    diff.grouped_ops(0)
        .into_iter()
        .filter_map(|group| {
            let first = group.first()?;
            let last = group.last()?;
            let old = first.old_range().start..last.old_range().end;
            let new = first.new_range().start..last.new_range().end;
            if old.is_empty() && new.is_empty() {
                return None;
            }
            Some(Hunk {
                lines: new,
                old_text: old_lines[old].concat(),
            })
        })
        .collect()
}
