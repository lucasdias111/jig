//! Find (⌘F) and Find and Replace (⌥⌘F) in the current file: a small
//! floating panel at the top right of the code, over the editor's own
//! search engine. The engine highlights the matches; the panel moves
//! between them, selecting each in the code, and replaces them.
//!
//! The search starts from the cursor. ↩ goes to the next match, ⇧↩ to the
//! previous; in the replace field ↩ replaces the current one, and ⌥⌘↩
//! replaces them all. A file under review is read-only, so the panel
//! won't replace anything then.

use std::ops::Range;

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::input::{
    EditorState, Enter, Escape, IndentInline, InputEvent, InputState, OutdentInline,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

pub const WIDTH: f32 = 420.;
const CONTEXT: &str = "JigFindPanel";

const CHEVRON_RIGHT: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="m9 18 6-6-6-6"/></svg>"#;
const CHEVRON_DOWN: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="m6 9 6 6 6-6"/></svg>"#;
const ARROW_UP: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="m5 12 7-7 7 7"/><path d="M12 19V5"/></svg>"#;
const ARROW_DOWN: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M12 5v14"/><path d="m19 12-7 7-7-7"/></svg>"#;
const CLOSE: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M18 6 6 18"/><path d="m6 6 12 12"/></svg>"#;

actions!(find_panel, [ReplaceAll, ToggleMatchCase]);

/// ⌥C as in Find in Files; ⌥⌘↩ as in VS Code.
pub fn key_bindings() -> Vec<KeyBinding> {
    vec![
        KeyBinding::new("alt-c", ToggleMatchCase, Some(CONTEXT)),
        KeyBinding::new("secondary-alt-enter", ReplaceAll, Some(CONTEXT)),
    ]
}

pub enum FindPanelEvent {
    /// Esc or the close button: close and go back to the code.
    Dismissed,
}

pub struct FindPanel {
    editor: Entity<EditorState>,
    query: Entity<InputState>,
    replacement: Entity<InputState>,
    replacing: bool,
    match_case: bool,
    /// Where the search starts: the cursor when the panel opened, then each
    /// match moved to, so that typing more stays at the same place.
    origin: usize,
    focus: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<FindPanelEvent> for FindPanel {}

impl Focusable for FindPanel {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.query.focus_handle(cx)
    }
}

impl FindPanel {
    /// A panel searching `editor` for `query`, typed and selected, from
    /// `origin`.
    pub fn new(
        editor: Entity<EditorState>,
        query: &str,
        origin: usize,
        replacing: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let field =
            |placeholder: &'static str, value: String, window: &mut Window, cx: &mut App| {
                cx.new(|cx| {
                    InputState::new(window, cx)
                        .placeholder(placeholder)
                        .default_value(value)
                })
            };
        let query_input = field("Find", query.to_string(), window, cx);
        let replacement = field("Replace", String::new(), window, cx);
        let subscriptions = vec![
            cx.subscribe_in(&query_input, window, |this, _, event, _, cx| {
                if let InputEvent::Change = event {
                    this.search(cx);
                }
            }),
            // The count changes as the code does.
            cx.observe(&editor, |_, _, cx| cx.notify()),
        ];
        let mut this = Self {
            editor,
            query: query_input,
            replacement,
            replacing,
            match_case: false,
            origin,
            focus: cx.focus_handle(),
            _subscriptions: subscriptions,
        };
        this.search(cx);
        if replacing {
            this.replacement
                .update(cx, |input, cx| input.focus(window, cx));
        } else {
            this.focus_query(window, cx);
        }
        this
    }

    pub fn query(&self, cx: &App) -> String {
        self.query.read(cx).value().to_string()
    }

    #[cfg(any(test, feature = "testing"))]
    pub fn is_replacing(&self) -> bool {
        self.replacing
    }

    /// Whether the keyboard is in the panel.
    pub fn has_focus(&self, window: &Window, cx: &App) -> bool {
        self.focus.contains_focused(window, cx)
    }

    /// Show again: back to the query, selected, with the replace field if
    /// asked for. Typed text, e.g. a fresh selection, replaces the query.
    pub fn reopen(
        &mut self,
        query: Option<&str>,
        replacing: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if replacing {
            self.replacing = true;
        }
        if let Some(query) = query.filter(|query| *query != self.query(cx)) {
            let query = query.to_string();
            self.query
                .update(cx, |input, cx| input.set_value(query, window, cx));
            self.search(cx);
        }
        if replacing {
            self.replacement
                .update(cx, |input, cx| input.focus(window, cx));
        } else {
            self.focus_query(window, cx);
        }
        cx.notify();
    }

    fn focus_query(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.query.update(cx, |input, cx| {
            input.focus(window, cx);
            input.select_all(window, cx);
        });
    }

    /// Look for the query from the origin and select the first match.
    fn search(&mut self, cx: &mut Context<Self>) {
        let query = self.query(cx);
        let (origin, case_insensitive) = (self.origin, !self.match_case);
        let found = self.editor.update(cx, |state, cx| {
            state.set_search_query(query, case_insensitive, cx);
            state.search_match_from(origin, cx)
        });
        self.select(found, cx);
    }

    /// Select `range` in the code, and search on from there.
    fn select(&mut self, range: Option<Range<usize>>, cx: &mut Context<Self>) {
        if let Some(range) = range {
            self.origin = range.start;
            self.editor
                .update(cx, |state, cx| state.set_selected_range(range, cx));
        }
        cx.notify();
    }

    fn next(&mut self, cx: &mut Context<Self>) {
        let found = self
            .editor
            .update(cx, |state, cx| state.next_search_match(cx));
        self.select(found, cx);
    }

    fn previous(&mut self, cx: &mut Context<Self>) {
        let found = self
            .editor
            .update(cx, |state, cx| state.previous_search_match(cx));
        self.select(found, cx);
    }

    fn can_replace(&self, cx: &App) -> bool {
        self.editor.read(cx).is_replaceable()
    }

    fn current_match(&self, cx: &App) -> Option<Range<usize>> {
        let matcher = &self.editor.read(cx).search_session().matcher;
        matcher
            .current()
            .and_then(|ix| matcher.matched_ranges().get(ix).cloned())
    }

    /// Replace the current match and go on to the next. Refused while the
    /// file is read-only.
    fn replace_one(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.can_replace(cx) {
            return;
        }
        let replacement = self.replacement.read(cx).value().to_string();
        let replaced = self.editor.update(cx, |state, cx| {
            state.replace_current_search_match(&replacement, window, cx)
        });
        if replaced {
            let next = self.current_match(cx);
            self.select(next, cx);
        }
    }

    /// Replace every match, as one undo step. Refused while the file is
    /// read-only.
    pub fn replace_all(&mut self, window: &mut Window, cx: &mut Context<Self>) -> usize {
        if !self.can_replace(cx) {
            return 0;
        }
        let replacement = self.replacement.read(cx).value().to_string();
        let count = self.editor.update(cx, |state, cx| {
            state.replace_all_search_matches(&replacement, window, cx)
        });
        cx.notify();
        count
    }

    fn toggle_replacing(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.replacing = !self.replacing;
        if self.replacing {
            self.replacement
                .update(cx, |input, cx| input.focus(window, cx));
        } else {
            self.query.update(cx, |input, cx| input.focus(window, cx));
        }
        cx.notify();
    }

    fn on_enter(&mut self, action: &Enter, window: &mut Window, cx: &mut Context<Self>) {
        cx.stop_propagation();
        if self.replacement.focus_handle(cx).is_focused(window) {
            self.replace_one(window, cx);
        } else if action.shift {
            self.previous(cx);
        } else {
            self.next(cx);
        }
    }

    fn on_escape(&mut self, _: &Escape, _: &mut Window, cx: &mut Context<Self>) {
        cx.stop_propagation();
        cx.emit(FindPanelEvent::Dismissed);
    }

    /// Tab moves between the two fields and never leaves the panel.
    fn on_tab(&mut self, _: &IndentInline, window: &mut Window, cx: &mut Context<Self>) {
        cx.stop_propagation();
        self.switch_field(window, cx);
    }

    fn on_shift_tab(&mut self, _: &OutdentInline, window: &mut Window, cx: &mut Context<Self>) {
        cx.stop_propagation();
        self.switch_field(window, cx);
    }

    fn switch_field(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.replacing {
            return;
        }
        let to = if self.query.focus_handle(cx).is_focused(window) {
            &self.replacement
        } else {
            &self.query
        };
        to.update(cx, |input, cx| input.focus(window, cx));
    }

    fn on_replace_all(&mut self, _: &ReplaceAll, window: &mut Window, cx: &mut Context<Self>) {
        if self.replacing {
            self.replace_all(window, cx);
        }
    }

    fn on_toggle_match_case(
        &mut self,
        _: &ToggleMatchCase,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.match_case = !self.match_case;
        self.search(cx);
    }

    /// `2 of 5`, or `No results` for a query that matches nothing.
    fn status(&self, cx: &App) -> String {
        let matcher = &self.editor.read(cx).search_session().matcher;
        match matcher.current() {
            Some(ix) => format!("{} of {}", ix + 1, matcher.len()),
            None if self.query(cx).is_empty() => String::new(),
            None => "No results".into(),
        }
    }
}

mod view;
