//! The problems language servers report (`textDocument/publishDiagnostics`),
//! kept per file whether or not it's open, so a file shows its problems as
//! soon as it opens. Each report replaces the file's last one; an empty one
//! clears it.

use std::collections::HashMap;
use std::ops::Range;

use gpui_kit::base::input::Diagnostic;
use gpui_kit::{App, Global};

use crate::domain::diagnostics::Published;
use crate::domain::positions::range;

/// Where in `text` each problem is, as the editor wants them: byte ranges,
/// clamped to the text.
pub fn entries(text: &str, published: &Published) -> Vec<(Range<usize>, Diagnostic)> {
    published
        .items
        .iter()
        .map(|item| {
            let range = range(text, item.range, published.encoding);
            (range.start.min(range.end)..range.end, item.clone().into())
        })
        .collect()
}

/// Every file's problems, as last reported.
#[derive(Default)]
pub struct Diagnostics {
    files: HashMap<String, Stored>,
    /// Counts reports, so a tab can tell whether its file's changed.
    reports: u64,
}

pub struct Stored {
    pub published: Published,
    /// Which report this was, so a tab can tell it has the latest.
    pub revision: u64,
}

impl Global for Diagnostics {}

impl Diagnostics {
    pub fn get(&self, key: &str) -> Option<&Stored> {
        self.files.get(key)
    }
}

/// Keep `published`, replacing the file's last report. Observers of
/// [`Diagnostics`] hear about it.
pub fn publish(published: Published, cx: &mut App) {
    let store = cx.default_global::<Diagnostics>();
    if published.items.is_empty() {
        store.files.remove(&published.key);
    } else {
        store.reports += 1;
        let revision = store.reports;
        store.files.insert(
            published.key.clone(),
            Stored {
                published,
                revision,
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::positions::Encoding;

    #[test]
    fn utf16_columns_become_byte_ranges() {
        let text = "let s = \"😀\"; bad\nnext\n";
        let item = |start: u32, end: u32| lsp_types::Diagnostic {
            range: lsp_types::Range::new(
                lsp_types::Position::new(0, start),
                lsp_types::Position::new(0, end),
            ),
            message: "x".into(),
            ..Default::default()
        };
        // `bad` starts after the emoji, two UTF-16 units but four bytes.
        let published = Published {
            key: String::new(),
            version: None,
            items: vec![item(14, 17), item(30, 99)],
            encoding: Encoding::Utf16,
        };
        let ranges: Vec<_> = entries(text, &published)
            .into_iter()
            .map(|(range, _)| range)
            .collect();
        let bad = text.find("bad").unwrap();
        // Past the end of the line stays on it.
        let line_end = text.find('\n').unwrap();
        assert_eq!(ranges, [bad..bad + 3, line_end..line_end]);
        assert_eq!(&text[ranges[0].clone()], "bad");

        let utf8 = Published {
            items: vec![item(16, 19)],
            encoding: Encoding::Utf8,
            ..published
        };
        assert_eq!(entries(text, &utf8)[0].0, bad..bad + 3);
    }
}
