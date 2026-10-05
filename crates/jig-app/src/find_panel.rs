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

use gpui_kit::component::input::{
    EditorState, Enter, Escape, IndentInline, Input, InputEvent, InputState, OutdentInline,
};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{ActiveTheme as _, Icon, h_flex, v_flex};
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

    #[cfg(test)]
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

    /// A small icon button that leaves the keyboard where it is.
    fn icon_button(
        &self,
        id: &'static str,
        icon: &'static [u8],
        tooltip: &'static str,
        enabled: bool,
        cx: &mut Context<Self>,
        on_click: impl Fn(&mut Self, &mut Window, &mut Context<Self>) + 'static,
    ) -> AnyElement {
        let theme = cx.theme();
        div()
            .id(id)
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .size(px(24.))
            .rounded(px(6.))
            .child(
                Icon::default()
                    .data(icon)
                    .size(px(14.))
                    .text_color(if enabled {
                        theme.muted_foreground
                    } else {
                        theme.muted_foreground.opacity(0.4)
                    }),
            )
            .when(enabled, |this| {
                this.cursor_pointer()
                    .hover(|s| s.bg(theme.foreground.opacity(0.06)))
                    .on_click(cx.listener(move |this, _, window, cx| on_click(this, window, cx)))
            })
            .tooltip(move |window, cx| Tooltip::new(tooltip).build(window, cx))
            .on_mouse_down(MouseButton::Left, |_, window, cx| {
                window.prevent_default();
                cx.stop_propagation();
            })
            .into_any_element()
    }

    fn text_button(
        &self,
        id: &'static str,
        label: &'static str,
        tooltip: &'static str,
        enabled: bool,
        cx: &mut Context<Self>,
        on_click: impl Fn(&mut Self, &mut Window, &mut Context<Self>) + 'static,
    ) -> AnyElement {
        let theme = cx.theme();
        div()
            .id(id)
            .flex_none()
            .h(px(24.))
            .px_2()
            .flex()
            .items_center()
            .rounded(px(6.))
            .text_size(px(12.))
            .border_1()
            .border_color(theme.foreground.opacity(0.1))
            .text_color(if enabled {
                theme.popover_foreground
            } else {
                theme.muted_foreground.opacity(0.5)
            })
            .child(label)
            .when(enabled, |this| {
                this.cursor_pointer()
                    .hover(|s| s.bg(theme.foreground.opacity(0.06)))
                    .on_click(cx.listener(move |this, _, window, cx| on_click(this, window, cx)))
            })
            .tooltip(move |window, cx| Tooltip::new(tooltip).build(window, cx))
            .on_mouse_down(MouseButton::Left, |_, window, cx| {
                window.prevent_default();
                cx.stop_propagation();
            })
            .into_any_element()
    }

    fn render_match_case(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let on = self.match_case;
        div()
            .id("find-match-case")
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .h(px(22.))
            .min_w(px(26.))
            .px_1()
            .rounded(px(5.))
            .text_size(px(11.5))
            .font_family(theme.mono_font_family.clone())
            .font_weight(FontWeight::SEMIBOLD)
            .cursor_pointer()
            .border_1()
            .when(on, |this| {
                this.bg(theme.primary.opacity(0.16))
                    .text_color(theme.primary)
                    .border_color(theme.primary.opacity(0.45))
            })
            .when(!on, |this| {
                this.text_color(theme.muted_foreground)
                    .border_color(transparent_black())
                    .hover(|s| s.bg(theme.foreground.opacity(0.06)))
            })
            .child("Cc")
            .tooltip(|window, cx| Tooltip::new("Match Case (⌥C)").build(window, cx))
            .on_mouse_down(MouseButton::Left, |_, window, cx| {
                window.prevent_default();
                cx.stop_propagation();
            })
            .on_click(cx.listener(|this, _, window, cx| {
                this.on_toggle_match_case(&ToggleMatchCase, window, cx)
            }))
            .into_any_element()
    }
}

impl Render for FindPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let has_matches = !self.editor.read(cx).search_session().matcher.is_empty();
        let can_replace = self.can_replace(cx);
        let status = self.status(cx);
        let toggle = self.icon_button(
            "find-toggle-replace",
            if self.replacing {
                CHEVRON_DOWN
            } else {
                CHEVRON_RIGHT
            },
            "Toggle Replace",
            true,
            cx,
            |this, window, cx| this.toggle_replacing(window, cx),
        );
        let match_case = self.render_match_case(cx);
        let previous = self.icon_button(
            "find-previous",
            ARROW_UP,
            "Previous Match (⇧↩)",
            has_matches,
            cx,
            |this, _, cx| this.previous(cx),
        );
        let next = self.icon_button(
            "find-next",
            ARROW_DOWN,
            "Next Match (↩)",
            has_matches,
            cx,
            |this, _, cx| this.next(cx),
        );
        let close = self.icon_button("find-close", CLOSE, "Close (esc)", true, cx, |_, _, cx| {
            cx.emit(FindPanelEvent::Dismissed)
        });
        let replace_row = self.replacing.then(|| {
            let enabled = can_replace && has_matches;
            let replace = self.text_button(
                "find-replace",
                "Replace",
                "Replace (↩)",
                enabled,
                cx,
                |this, window, cx| this.replace_one(window, cx),
            );
            let replace_all = self.text_button(
                "find-replace-all",
                "All",
                "Replace All (⌥⌘↩)",
                enabled,
                cx,
                |this, window, cx| {
                    this.replace_all(window, cx);
                },
            );
            h_flex()
                .gap_1()
                .pl(px(28.))
                .child(field(&self.replacement, cx))
                .child(replace)
                .child(replace_all)
        });
        let theme = cx.theme();
        let read_only = (self.replacing && !can_replace).then(|| {
            div()
                .pl(px(32.))
                .pb_0p5()
                .child(jig_commands::surface::hint(
                    "Read-only while a change is under review. Accept or reject it first.",
                    cx,
                ))
        });

        let panel = jig_commands::surface::panel(cx)
            .track_focus(&self.focus)
            .key_context(CONTEXT)
            .capture_action(cx.listener(Self::on_enter))
            .capture_action(cx.listener(Self::on_escape))
            .capture_action(cx.listener(Self::on_tab))
            .capture_action(cx.listener(Self::on_shift_tab))
            .on_action(cx.listener(Self::on_replace_all))
            .on_action(cx.listener(Self::on_toggle_match_case))
            .w(px(WIDTH))
            .p_1()
            .child(
                v_flex()
                    .gap_1()
                    .child(
                        h_flex()
                            .gap_1()
                            .child(toggle)
                            .child(field(&self.query, cx).child(match_case))
                            .child(
                                div()
                                    .flex_none()
                                    .min_w(px(64.))
                                    .text_size(px(11.5))
                                    .text_color(if has_matches || status.is_empty() {
                                        theme.muted_foreground
                                    } else {
                                        theme.danger
                                    })
                                    .child(status),
                            )
                            .child(previous)
                            .child(next)
                            .child(close),
                    )
                    .children(replace_row)
                    .children(read_only),
            );
        jig_commands::motion::pop_in(panel, "jig-find-panel")
    }
}

/// One of the panel's two text fields, in a faint well.
fn field(input: &Entity<InputState>, cx: &App) -> Div {
    let theme = cx.theme();
    h_flex()
        .flex_1()
        .min_w_0()
        .h(px(28.))
        .pl_2()
        .pr_0p5()
        .gap_1()
        .rounded(px(7.))
        .bg(theme.foreground.opacity(0.05))
        .text_size(px(13.))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .child(Input::new(input).appearance(false).cleanable(false)),
        )
}
