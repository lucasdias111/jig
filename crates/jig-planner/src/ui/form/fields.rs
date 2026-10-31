//! The form's fields that can link to code, and their state: one line or
//! several.

use gpui_kit::component::input::{InputState, TextareaState};
use gpui_kit::*;

/// A field that can link to code.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Field {
    Description,
    NewTodo,
    /// A todo, by its key.
    Todo(usize),
}

/// A field's state: one line or several.
pub enum FieldState {
    Line(Entity<InputState>),
    Area(Entity<TextareaState>),
}

impl FieldState {
    pub fn text_and_cursor(&self, cx: &App) -> (String, usize) {
        match self {
            FieldState::Line(state) => {
                let state = state.read(cx);
                (state.value().to_string(), state.cursor())
            }
            FieldState::Area(state) => {
                let state = state.read(cx);
                (state.value().to_string(), state.cursor())
            }
        }
    }

    /// Put `text` in place of `range`, as one undo step, the cursor after.
    pub fn replace(
        &self,
        range: std::ops::Range<usize>,
        text: &str,
        window: &mut Window,
        cx: &mut App,
    ) {
        match self {
            FieldState::Line(state) => state.update(cx, |state, cx| {
                state.set_selected_range(range, cx);
                state.replace(text.to_string(), window, cx);
            }),
            FieldState::Area(state) => state.update(cx, |state, cx| {
                state.set_selected_range(range, cx);
                state.replace(text.to_string(), window, cx);
            }),
        }
    }

    pub fn focus(&self, window: &mut Window, cx: &mut App) {
        match self {
            FieldState::Line(state) => state.update(cx, |state, cx| state.focus(window, cx)),
            FieldState::Area(state) => state.update(cx, |state, cx| state.focus(window, cx)),
        }
    }
}
