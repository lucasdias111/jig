//! The line shortcuts in the code editor (move, duplicate, delete, select
//! and open lines) and the code font size.

use gpui_kit::*;
use jig_editor::EditorHandle;

use super::{
    DeleteLine, DuplicateLine, InsertLineAbove, InsertLineBelow, MoveLineDown, MoveLineUp,
    ResetZoom, SelectLine, Workspace, ZoomIn, ZoomOut,
};
use crate::lines::{self, LineEdit};
use crate::settings::{self, DEFAULT_FONT_SIZE, MAX_FONT_SIZE, MIN_FONT_SIZE};

impl Workspace {
    /// Make `edit` as one undo step, if the buffer can be changed now.
    fn line_edit(
        &mut self,
        edit: impl FnOnce(&str, std::ops::Range<usize>) -> Option<LineEdit>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.home || self.previewing() {
            return;
        }
        let text = self.editor().text(cx);
        let Some(edit) = edit(&text, self.editor().selection(cx)) else {
            return;
        };
        self.editor().apply_edit(edit.range, &edit.text, window, cx);
        self.editor().select(edit.selection, cx);
    }

    pub(super) fn move_line_up(
        &mut self,
        _: &MoveLineUp,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.line_edit(
            |text, selection| lines::move_lines(text, selection, true),
            window,
            cx,
        );
    }

    pub(super) fn move_line_down(
        &mut self,
        _: &MoveLineDown,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.line_edit(
            |text, selection| lines::move_lines(text, selection, false),
            window,
            cx,
        );
    }

    pub(super) fn duplicate_line(
        &mut self,
        _: &DuplicateLine,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.line_edit(
            |text, selection| Some(lines::duplicate_lines(text, selection)),
            window,
            cx,
        );
    }

    pub(super) fn delete_line(
        &mut self,
        _: &DeleteLine,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.line_edit(
            |text, selection| Some(lines::delete_lines(text, selection)),
            window,
            cx,
        );
    }

    pub(super) fn insert_line_below(
        &mut self,
        _: &InsertLineBelow,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.line_edit(
            |text, selection| Some(lines::open_line(text, selection, true)),
            window,
            cx,
        );
    }

    pub(super) fn insert_line_above(
        &mut self,
        _: &InsertLineAbove,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.line_edit(
            |text, selection| Some(lines::open_line(text, selection, false)),
            window,
            cx,
        );
    }

    pub(super) fn select_line(&mut self, _: &SelectLine, _: &mut Window, cx: &mut Context<Self>) {
        if self.home {
            return;
        }
        let text = self.editor().text(cx);
        let range = lines::select_lines(&text, self.editor().selection(cx));
        self.editor().select(range, cx);
    }

    pub(super) fn zoom_in(&mut self, _: &ZoomIn, _: &mut Window, cx: &mut Context<Self>) {
        change_font_size(1., cx);
    }

    pub(super) fn zoom_out(&mut self, _: &ZoomOut, _: &mut Window, cx: &mut Context<Self>) {
        change_font_size(-1., cx);
    }

    pub(super) fn reset_zoom(&mut self, _: &ResetZoom, _: &mut Window, cx: &mut Context<Self>) {
        settings::update(cx, |s| s.appearance.font_size = DEFAULT_FONT_SIZE);
    }
}

/// A point bigger or smaller, kept to whole and half points.
fn change_font_size(by: f32, cx: &mut App) {
    settings::update(cx, |s| {
        let size = ((s.appearance.font_size + by) * 2.).round() / 2.;
        s.appearance.font_size = size.clamp(MIN_FONT_SIZE, MAX_FONT_SIZE);
    });
}
