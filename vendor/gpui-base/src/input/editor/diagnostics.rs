use std::{
    cmp::Ordering,
    ops::{Deref, Range},
};

use gpui::SharedString;
use ropey::Rope;
use sum_tree::{Bias, SeekTarget, SumTree};

use crate::input::{Position, RopeExt as _};

pub type DiagnosticRelatedInformation = lsp_types::DiagnosticRelatedInformation;
pub(super) type CodeDescription = lsp_types::CodeDescription;
pub type RelatedInformation = lsp_types::DiagnosticRelatedInformation;
pub type DiagnosticTag = lsp_types::DiagnosticTag;

#[derive(Debug, Eq, PartialEq, Clone, Default)]
pub struct Diagnostic {
    /// The range [`Position`] at which the message applies.
    ///
    /// This is the column, character range within a single line.
    pub range: Range<Position>,

    /// The diagnostic's severity. Can be omitted. If omitted it is up to the
    /// client to interpret diagnostics as error, warning, info or hint.
    pub severity: DiagnosticSeverity,

    /// The diagnostic's code. Can be omitted.
    pub code: Option<SharedString>,

    pub code_description: Option<CodeDescription>,

    /// A human-readable string describing the source of this
    /// diagnostic, e.g. 'typescript' or 'super lint'.
    pub source: Option<SharedString>,

    /// The diagnostic's message.
    pub message: SharedString,

    /// An array of related diagnostic information, e.g. when symbol-names within
    /// a scope collide all definitions can be marked via this property.
    pub related_information: Option<Vec<DiagnosticRelatedInformation>>,

    /// Additional metadata about the diagnostic.
    pub tags: Option<Vec<DiagnosticTag>>,

    /// A data entry field that is preserved between a `textDocument/publishDiagnostics`
    /// notification and `textDocument/codeAction` request.
    ///
    /// @since 3.16.0
    pub data: Option<serde_json::Value>,
}

impl From<lsp_types::Diagnostic> for Diagnostic {
    fn from(value: lsp_types::Diagnostic) -> Self {
        Self {
            range: value.range.start..value.range.end,
            severity: value
                .severity
                .map(Into::into)
                .unwrap_or(DiagnosticSeverity::Info),
            code: value.code.map(|c| match c {
                lsp_types::NumberOrString::Number(n) => SharedString::from(n.to_string()),
                lsp_types::NumberOrString::String(s) => SharedString::from(s),
            }),
            code_description: value.code_description,
            source: value.source.map(|s| s.into()),
            message: value.message.into(),
            related_information: value.related_information,
            tags: value.tags,
            data: value.data,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DiagnosticSeverity {
    #[default]
    Hint,
    Error,
    Warning,
    Info,
}

impl From<lsp_types::DiagnosticSeverity> for DiagnosticSeverity {
    fn from(value: lsp_types::DiagnosticSeverity) -> Self {
        match value {
            lsp_types::DiagnosticSeverity::ERROR => Self::Error,
            lsp_types::DiagnosticSeverity::WARNING => Self::Warning,
            lsp_types::DiagnosticSeverity::INFORMATION => Self::Info,
            lsp_types::DiagnosticSeverity::HINT => Self::Hint,
            _ => Self::Info, // Default to Info if unknown
        }
    }
}

impl Diagnostic {
    pub fn new(range: Range<impl Into<Position>>, message: impl Into<SharedString>) -> Self {
        Self {
            range: range.start.into()..range.end.into(),
            message: message.into(),
            ..Default::default()
        }
    }

    pub fn with_severity(mut self, severity: impl Into<DiagnosticSeverity>) -> Self {
        self.severity = severity.into();
        self
    }

    pub fn with_code(mut self, code: impl Into<SharedString>) -> Self {
        self.code = Some(code.into());
        self
    }

    pub fn with_source(mut self, source: impl Into<SharedString>) -> Self {
        self.source = Some(source.into());
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DiagnosticEntry {
    /// The byte range of the diagnostic in the rope.
    pub range: Range<usize>,
    pub diagnostic: Diagnostic,
}

impl Deref for DiagnosticEntry {
    type Target = Diagnostic;

    fn deref(&self) -> &Self::Target {
        &self.diagnostic
    }
}

#[derive(Debug, Default, Clone)]
pub struct DiagnosticSummary {
    count: usize,
    start: usize,
    end: usize,
}

impl sum_tree::Item for DiagnosticEntry {
    type Summary = DiagnosticSummary;
    fn summary(&self, _cx: &()) -> Self::Summary {
        DiagnosticSummary {
            count: 1,
            start: self.range.start,
            end: self.range.end,
        }
    }
}

impl sum_tree::Summary for DiagnosticSummary {
    type Context<'a> = &'a ();
    fn zero(_: Self::Context<'_>) -> Self {
        DiagnosticSummary {
            count: 0,
            start: usize::MIN,
            end: usize::MIN,
        }
    }

    fn add_summary(&mut self, other: &Self, _: Self::Context<'_>) {
        self.start = other.start;
        // Jig patch: the furthest end so far, not the last one, so seeking
        // never skips a long diagnostic followed by shorter ones.
        self.end = self.end.max(other.end);
        self.count += other.count;
    }
}

/// For seeking by byte range.
impl SeekTarget<'_, DiagnosticSummary, DiagnosticSummary> for usize {
    fn cmp(&self, other: &DiagnosticSummary, _: &()) -> Ordering {
        if *self < other.start {
            Ordering::Less
        } else if *self > other.end {
            Ordering::Greater
        } else {
            Ordering::Equal
        }
    }
}

#[derive(Debug, Clone)]
pub struct DiagnosticSet {
    text: Rope,
    diagnostics: SumTree<DiagnosticEntry>,
}

impl DiagnosticSet {
    pub fn new(text: &Rope) -> Self {
        Self {
            text: text.clone(),
            diagnostics: SumTree::new(&()),
        }
    }

    pub fn reset(&mut self, text: &Rope) {
        self.text = text.clone();
        self.clear();
    }

    pub fn push(&mut self, diagnostic: impl Into<Diagnostic>) {
        let diagnostic = diagnostic.into();
        let start = self.text.position_to_offset(&diagnostic.range.start);
        let end = self.text.position_to_offset(&diagnostic.range.end);

        self.diagnostics.push(
            DiagnosticEntry {
                range: start..end,
                diagnostic,
            },
            &(),
        );
    }

    pub fn extend<D, I>(&mut self, diagnostics: D)
    where
        D: IntoIterator<Item = I>,
        I: Into<Diagnostic>,
    {
        for diagnostic in diagnostics {
            self.push(diagnostic.into());
        }
    }

    /// Jig patch: replace every diagnostic with `entries`, whose byte
    /// ranges in `text` are already worked out (a language server counts
    /// columns its own way). Positions are filled in from the ranges.
    pub fn set_entries(&mut self, text: &Rope, entries: Vec<(Range<usize>, Diagnostic)>) {
        self.text = text.clone();
        let entries = entries
            .into_iter()
            .map(|(range, diagnostic)| {
                let end = range.end.min(text.len());
                DiagnosticEntry {
                    range: range.start.min(end)..end,
                    diagnostic,
                }
            })
            .collect();
        self.rebuild(entries);
    }

    /// Jig patch: `range` (bytes) was replaced by `new_len` bytes of text.
    /// Diagnostics after it move with the text, ones around it stretch or
    /// shrink, and ones whose text was replaced entirely go. Call
    /// [`Self::set_text`] once the edits are done.
    pub fn adjust_for_edit(&mut self, range: &Range<usize>, new_len: usize) {
        if self.is_empty() {
            return;
        }
        let shift = |offset: usize| offset - range.end + range.start + new_len;
        let entries = self
            .diagnostics
            .iter()
            .filter_map(|entry| {
                let Range { start, end } = entry.range;
                // A point keeps its place unless its spot was replaced.
                if start == end {
                    let at = if start <= range.start {
                        start
                    } else if start >= range.end {
                        shift(start)
                    } else {
                        return None;
                    };
                    return Some(DiagnosticEntry {
                        range: at..at,
                        diagnostic: entry.diagnostic.clone(),
                    });
                }
                // Text typed right before it isn't in it; text replaced
                // inside it is.
                let new_start = if start < range.start {
                    start
                } else if start >= range.end {
                    shift(start)
                } else {
                    range.start + new_len
                };
                let new_end = if end <= range.start {
                    end
                } else if end >= range.end {
                    shift(end)
                } else {
                    range.start
                };
                (new_start < new_end).then(|| DiagnosticEntry {
                    range: new_start..new_end,
                    diagnostic: entry.diagnostic.clone(),
                })
            })
            .collect();
        self.rebuild(entries);
    }

    /// Jig patch: the text after edits, so positions match it again.
    pub fn set_text(&mut self, text: &Rope) {
        self.text = text.clone();
        let entries = self.diagnostics.iter().cloned().collect();
        self.rebuild(entries);
    }

    /// Jig patch: the tree from `entries` in order, with positions from
    /// their byte ranges.
    fn rebuild(&mut self, mut entries: Vec<DiagnosticEntry>) {
        entries.sort_by_key(|entry| (entry.range.start, entry.range.end));
        let text = &self.text;
        for entry in &mut entries {
            let start = entry.range.start.min(text.len());
            let end = entry.range.end.min(text.len());
            entry.diagnostic.range = text.offset_to_position(start)..text.offset_to_position(end);
        }
        self.diagnostics = SumTree::from_iter(entries, &());
    }

    pub fn len(&self) -> usize {
        self.diagnostics.summary().count
    }

    pub fn clear(&mut self) {
        self.diagnostics = SumTree::new(&());
    }

    pub fn is_empty(&self) -> bool {
        self.diagnostics.is_empty()
    }

    pub fn range(&self, range: Range<usize>) -> impl Iterator<Item = &DiagnosticEntry> {
        let mut cursor = self.diagnostics.cursor::<DiagnosticSummary>(&());
        cursor.seek(&range.start, Bias::Left);
        std::iter::from_fn(move || {
            if let Some(entry) = cursor.item() {
                if entry.range.start < range.end {
                    cursor.next();
                    return Some(entry);
                }
            }
            None
        })
    }

    pub fn for_offset(&self, offset: usize) -> Option<&DiagnosticEntry> {
        self.range(offset..offset + 1).next()
    }

    #[allow(unused)]
    pub fn iter(&self) -> impl Iterator<Item = &DiagnosticEntry> {
        self.diagnostics.iter()
    }
}

#[cfg(test)]
mod tests {
    use crate::input::Position;

    #[test]
    fn test_diagnostic() {
        use ropey::Rope;

        use super::{Diagnostic, DiagnosticSet, DiagnosticSeverity};

        let text = Rope::from("Hello, 你好warld!\nThis is a test.\nGoodbye, world!");
        let mut diagnostics = DiagnosticSet::new(&text);

        diagnostics.push(
            Diagnostic::new(
                Position::new(0, 7)..Position::new(0, 17),
                "Spelling mistake",
            )
            .with_severity(DiagnosticSeverity::Warning),
        );
        diagnostics.push(
            Diagnostic::new(Position::new(2, 9)..Position::new(2, 14), "Syntax error")
                .with_severity(DiagnosticSeverity::Error),
        );

        assert_eq!(diagnostics.len(), 2);
        let items = diagnostics.iter().collect::<Vec<_>>();

        assert_eq!(items[0].message.as_str(), "Spelling mistake");
        assert_eq!(items[0].range, 7..19);

        assert_eq!(items[1].message.as_str(), "Syntax error");
        assert_eq!(items[1].range, 45..50);

        let items = diagnostics.range(6..48).collect::<Vec<_>>();
        assert_eq!(items.len(), 2);

        let item = diagnostics.for_offset(10).unwrap();
        assert_eq!(item.message.as_str(), "Spelling mistake");

        let item = diagnostics.for_offset(30);
        assert!(item.is_none());

        let item = diagnostics.for_offset(46).unwrap();
        assert_eq!(item.message.as_str(), "Syntax error");

        // Jig patch: a long diagnostic before a short one is still found.
        let mut nested = DiagnosticSet::new(&text);
        nested.push(Diagnostic::new(
            Position::new(0, 0)..Position::new(2, 4),
            "Long",
        ));
        nested.push(Diagnostic::new(
            Position::new(0, 1)..Position::new(0, 2),
            "Short",
        ));
        let found = nested.range(40..41).next().unwrap();
        assert_eq!(found.message.as_str(), "Long");

        diagnostics.push(
            Diagnostic::new(Position::new(1, 5)..Position::new(1, 7), "Info message")
                .with_severity(DiagnosticSeverity::Info),
        );
        assert_eq!(diagnostics.len(), 3);

        diagnostics.clear();
        assert_eq!(diagnostics.len(), 0);
    }

    /// Jig patch: diagnostics follow edits.
    #[test]
    fn test_adjust_for_edit() {
        use ropey::Rope;

        use super::{Diagnostic, DiagnosticSet};

        let text = Rope::from("let a = b;\nlet c = d;\n");
        let mut set = DiagnosticSet::new(&text);
        let entries = |set: &DiagnosticSet| {
            set.iter()
                .map(|e| (e.range.clone(), e.message.to_string()))
                .collect::<Vec<_>>()
        };
        set.set_entries(
            &text,
            vec![
                (
                    19..20,
                    Diagnostic::new(Position::new(0, 0)..Position::new(0, 0), "d"),
                ),
                (
                    8..9,
                    Diagnostic::new(Position::new(0, 0)..Position::new(0, 0), "b"),
                ),
            ],
        );
        assert_eq!(entries(&set), [(8..9, "b".into()), (19..20, "d".into())]);
        assert_eq!(
            set.iter().nth(1).unwrap().diagnostic.range.start,
            Position::new(1, 8)
        );

        // Typing before both moves them; typing right before one doesn't
        // stretch it.
        set.adjust_for_edit(&(0..0), 2);
        set.adjust_for_edit(&(10..10), 1);
        assert_eq!(entries(&set), [(11..12, "b".into()), (22..23, "d".into())]);

        // Replacing part of one shrinks it to what's left; replacing all
        // of one removes it.
        set.adjust_for_edit(&(21..22), 3);
        assert_eq!(entries(&set), [(11..12, "b".into()), (24..25, "d".into())]);
        set.adjust_for_edit(&(24..25), 3);
        assert_eq!(entries(&set), [(11..12, "b".into())]);

        // Positions follow once the text is set.
        let text = Rope::from("  let a = xb;\nlet c = eee;\n");
        set.set_text(&text);
        assert_eq!(
            set.iter().next().unwrap().diagnostic.range.start,
            Position::new(0, 11)
        );
    }
}
