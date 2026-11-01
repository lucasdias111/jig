//! Problems language servers report, in the code: squiggles coloured by
//! severity, a small panel with the message on hover, error and warning
//! counts in the status bar, and keys to step from one problem to the next.
//!
//! The editor keeps each tab's problems and moves them with edits until the
//! server reports again (a vendored-editor patch). Without a server nothing
//! shows.

use std::ops::Range;
use std::rc::Rc;
use std::time::Duration;

use gpui_kit::base::input::{DiagnosticEntry, DiagnosticSeverity};
use gpui_kit::component::input::EditorState;
use gpui_kit::component::{ActiveTheme as _, Icon, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use jig_editor::EditorHandle;

use super::quick_fix::PanelFixes;
use super::{NextProblem, PreviousProblem, Workspace};
use jig_lsp::lsp::diagnostics::{self, Diagnostics};

mod problem_hover;
pub(super) use problem_hover::{ProblemHover, problem_row};

/// Lucide "circle-x".
const ERROR_ICON: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="10"/><path d="m15 9-6 6"/><path d="m9 9 6 6"/></svg>"#;
/// Lucide "triangle-alert".
const WARNING_ICON: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="m21.73 18-8-14a2 2 0 0 0-3.48 0l-8 14A2 2 0 0 0 4 21h16a2 2 0 0 0 1.73-3"/><path d="M12 9v4"/><path d="M12 17h.01"/></svg>"#;
/// Lucide "info".
const INFO_ICON: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="10"/><path d="M12 16v-4"/><path d="M12 8h.01"/></svg>"#;

const PANEL_MAX_WIDTH: f32 = 480.;
/// How long a panel stays once the pointer leaves what it's about, so the
/// pointer can reach it.
pub(super) const LINGER: Duration = Duration::from_millis(350);

/// A tab's problems: which file they're kept under and which report it
/// shows.
pub(super) struct Problems {
    /// The file's name in [`Diagnostics`]; `None` while untitled.
    key: Option<String>,
    /// The report the editor has, so a new one is told apart.
    shown: Option<u64>,
    pub(super) hover: Entity<ProblemHover>,
}

impl Problems {
    /// Problems for `path`'s editor `state`, with whatever was reported for
    /// the file before it opened.
    pub(super) fn new(
        state: &Entity<EditorState>,
        path: Option<&std::path::Path>,
        workspace: WeakEntity<Workspace>,
        cx: &mut App,
    ) -> Self {
        state.update(cx, |state, _| state.set_own_diagnostic_popover(true));
        let hover = cx.new(|cx| ProblemHover::new(state.clone(), workspace, cx));
        let mut problems = Self {
            key: path.map(diagnostics::path_key),
            shown: None,
            hover,
        };
        problems.show(state, true, cx);
        problems
    }

    /// The file is now `path`, e.g. after Save As.
    pub(super) fn rename(
        &mut self,
        state: &Entity<EditorState>,
        path: &std::path::Path,
        cx: &mut App,
    ) {
        let key = diagnostics::path_key(path);
        if self.key.as_ref() != Some(&key) {
            self.key = Some(key);
            self.show(state, true, cx);
        }
    }

    /// What the server last reported for the file, as it sent it.
    pub(super) fn reported(&self, cx: &App) -> Vec<lsp_types::Diagnostic> {
        cx.try_global::<Diagnostics>()
            .zip(self.key.as_deref())
            .and_then(|(store, key)| store.get(key))
            .map(|stored| stored.published.items.clone())
            .unwrap_or_default()
    }

    /// Give the editor the latest report, if it hasn't got it. Returns
    /// whether anything changed.
    fn show(&mut self, state: &Entity<EditorState>, force: bool, cx: &mut App) -> bool {
        // Read without `default_global`, which would tell observers (this
        // one's caller among them) that the store changed.
        let store = cx.try_global::<Diagnostics>();
        let stored = store
            .zip(self.key.as_deref())
            .and_then(|(s, key)| s.get(key));
        let revision = stored.map(|stored| stored.revision);
        if !force && revision == self.shown {
            return false;
        }
        let published = stored.map(|stored| stored.published.clone());
        self.shown = revision;
        state.update(cx, |state, cx| {
            let text = state.text().clone();
            let entries = published
                .map(|published| diagnostics::entries(&text.to_string(), &published))
                .unwrap_or_default();
            if let Some(set) = state.diagnostics_mut() {
                set.set_entries(&text, entries);
            }
            cx.notify();
        });
        true
    }
}

/// Counts of a file's problems, for the status bar.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct Counts {
    pub(super) errors: usize,
    pub(super) warnings: usize,
}

impl Workspace {
    /// A server reported problems: show them in the tabs they're for.
    pub(super) fn problems_reported(&mut self, cx: &mut Context<Self>) {
        let mut changed = false;
        for tab in &mut self.tabs {
            let state = tab.editor.state().clone();
            changed |= tab.problems.show(&state, false, cx);
        }
        if changed {
            cx.notify();
        }
    }

    /// The current file's problems, in order.
    fn problem_entries(&self, cx: &App) -> Vec<DiagnosticEntry> {
        self.editor()
            .state()
            .read(cx)
            .diagnostics()
            .map(|set| set.iter().cloned().collect())
            .unwrap_or_default()
    }

    pub(super) fn problem_counts(&self, cx: &App) -> Counts {
        let mut counts = Counts::default();
        for entry in self.problem_entries(cx) {
            match entry.severity {
                DiagnosticSeverity::Error => counts.errors += 1,
                DiagnosticSeverity::Warning => counts.warnings += 1,
                DiagnosticSeverity::Info | DiagnosticSeverity::Hint => {}
            }
        }
        counts
    }

    pub(super) fn next_problem(
        &mut self,
        _: &NextProblem,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.go_to_problem(true, window, cx);
    }

    pub(super) fn previous_problem(
        &mut self,
        _: &PreviousProblem,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.go_to_problem(false, window, cx);
    }

    /// Put the cursor on the next (or previous) problem in the file, round
    /// the end, and show its message.
    fn go_to_problem(&mut self, forward: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.home {
            return;
        }
        let entries = self.problem_entries(cx);
        let cursor = self.editor().cursor(cx);
        let Some(ix) = step(&entries, cursor, forward) else {
            return;
        };
        let entry = entries[ix].clone();
        self.remember_location(cx);
        let editor = self.editor().clone();
        editor.select(entry.range.start..entry.range.start, cx);
        editor.focus(window, cx);
        self.tab()
            .problems
            .hover
            .update(cx, |hover, cx| hover.pin(entry, cx));
        cx.notify();
    }

    /// Esc: put away a problem's message shown by the keys.
    pub(super) fn close_problem(&mut self, cx: &mut Context<Self>) -> bool {
        self.tab()
            .problems
            .hover
            .update(cx, |hover, cx| hover.unpin(cx))
    }

    /// The problems on the lines `target` covers in `text`, for fixes that
    /// fix them.
    pub(super) fn problems_on_lines(
        &self,
        text: &str,
        target: Range<usize>,
        cx: &App,
    ) -> Vec<jig_ai::agent::Diagnostic> {
        on_lines(&self.problem_entries(cx), text, target)
    }

    /// The status bar's error and warning counts, which step through them
    /// when clicked. Only for files a language server checks.
    pub(super) fn render_problem_counts(&self, cx: &Context<Self>) -> Option<AnyElement> {
        self.tab().lsp.as_ref()?;
        let theme = cx.theme();
        let counts = self.problem_counts(cx);
        let count = |icon: &'static [u8], n: usize, color: Hsla| {
            h_flex()
                .gap_1()
                .child(
                    Icon::default()
                        .data(icon)
                        .size(px(12.))
                        .text_color(if n > 0 { color } else { theme.muted_foreground }),
                )
                .child(n.to_string())
        };
        Some(
            h_flex()
                .id("problem-counts")
                .debug_selector(|| "problem-counts".into())
                .gap_2()
                .px_1()
                .rounded(px(4.))
                .hover(|s| s.bg(theme.foreground.opacity(0.06)))
                .child(count(
                    ERROR_ICON,
                    counts.errors,
                    severity_color(DiagnosticSeverity::Error, cx),
                ))
                .child(count(
                    WARNING_ICON,
                    counts.warnings,
                    severity_color(DiagnosticSeverity::Warning, cx),
                ))
                .on_click(cx.listener(|this, _, window, cx| this.go_to_problem(true, window, cx)))
                .into_any_element(),
        )
    }
}

/// The problem to go to from `cursor`: the next one starting after it, or
/// the last one starting before it, round the end either way. Problems that
/// start at the same place are one stop.
fn step(entries: &[DiagnosticEntry], cursor: usize, forward: bool) -> Option<usize> {
    if entries.is_empty() {
        return None;
    }
    // Entries are in order of where they start.
    let found = if forward {
        entries.iter().position(|e| e.range.start > cursor)
    } else {
        entries.iter().rposition(|e| e.range.start < cursor)
    };
    let ix = found.unwrap_or(if forward { 0 } else { entries.len() - 1 });
    // The first, and so the most telling, of those starting there.
    let start = entries[ix].range.start;
    entries.iter().position(|e| e.range.start == start)
}

fn on_lines(
    entries: &[DiagnosticEntry],
    text: &str,
    target: Range<usize>,
) -> Vec<jig_ai::agent::Diagnostic> {
    let start = text[..target.start].rfind('\n').map_or(0, |ix| ix + 1);
    // A selection of whole lines ends after the last one's newline.
    let last = if target.end > target.start && text[..target.end].ends_with('\n') {
        target.end - 1
    } else {
        target.end
    };
    let end = text[last..].find('\n').map_or(text.len(), |ix| last + ix);
    entries
        .iter()
        .filter(|e| e.range.start <= end && e.range.end >= start)
        .map(|e| {
            let at = e.range.start.max(start).min(text.len());
            let mut message = e.message.to_string();
            let origin: Vec<&str> = [e.source.as_deref(), e.code.as_deref()]
                .into_iter()
                .flatten()
                .collect();
            if !origin.is_empty() {
                message.push_str(&format!(" ({})", origin.join(" ")));
            }
            jig_ai::agent::Diagnostic {
                line: text[..at].matches('\n').count() + 1,
                severity: severity_name(e.severity).into(),
                message,
            }
        })
        .collect()
}

fn severity_name(severity: DiagnosticSeverity) -> &'static str {
    match severity {
        DiagnosticSeverity::Error => "error",
        DiagnosticSeverity::Warning => "warning",
        DiagnosticSeverity::Info => "info",
        DiagnosticSeverity::Hint => "hint",
    }
}

/// The theme's colour for `severity`, the squiggles' too.
fn severity_color(severity: DiagnosticSeverity, cx: &App) -> Hsla {
    let status = &cx.theme().highlight_theme.style.status;
    match severity {
        DiagnosticSeverity::Error => status.error(cx),
        DiagnosticSeverity::Warning => status.warning(cx),
        DiagnosticSeverity::Info => status.info(cx),
        DiagnosticSeverity::Hint => status.hint(cx),
    }
}

#[cfg(test)]
mod tests;
