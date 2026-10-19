//! Go Back and Go Forward (⌃- and ⌃⇧-): jumps (a task's link, Go to
//! Definition, Go to Line, a problem, a search result) remember where the
//! cursor was, so you can go back and forth between them. The Map's
//! places are too: entering it, each level stepped into and each
//! re-centring; going back to one shows the Map there. A mouse's back and
//! forward buttons do the same.

use std::path::{Path, PathBuf};

use gpui_kit::*;
use jig_editor::EditorHandle as _;

use super::tabs::canonical;
use super::{GoBack, GoForward, Workspace};
use crate::map::view::Level;

/// More than anyone goes back through.
const MAX_PLACES: usize = 100;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Place {
    Code {
        path: PathBuf,
        /// The cursor, as a byte offset. Edits since may have moved what
        /// was there; it's kept within the file.
        offset: usize,
    },
    /// A level of the Map.
    Map(Level),
}

impl Place {
    /// Whether it can still be gone to: its file is there.
    fn exists(&self) -> bool {
        match self {
            Place::Code { path, .. } => path.is_file(),
            Place::Map(_) => true,
        }
    }
}

#[derive(Default)]
pub(super) struct History {
    back: Vec<Place>,
    forward: Vec<Place>,
}

impl History {
    fn push(stack: &mut Vec<Place>, place: Place) {
        if stack.last() != Some(&place) {
            stack.push(place);
        }
        if stack.len() > MAX_PLACES {
            stack.remove(0);
        }
    }
}

impl Workspace {
    /// Whether the current tab shows `path`.
    pub(super) fn showing(&self, path: &Path) -> bool {
        !self.home
            && self
                .document()
                .path
                .as_deref()
                .is_some_and(|open| canonical(open) == canonical(path))
    }

    /// Where the cursor is, in a saved file, or the Map's level.
    fn here(&self, cx: &App) -> Option<Place> {
        if let Some(view) = self.map_showing() {
            return Some(Place::Map(view.read(cx).level().clone()));
        }
        if self.home {
            return None;
        }
        Some(Place::Code {
            path: self.document().path.clone()?,
            offset: self.editor().cursor(cx),
        })
    }

    /// The Map moved on from `level`: Go Back returns to it.
    pub(super) fn remember_map(&mut self, level: Level) {
        self.remember(Place::Map(level));
    }

    /// About to jump somewhere: remember where the cursor is, for Go Back.
    pub(super) fn remember_location(&mut self, cx: &App) {
        if let Some(here) = self.here(cx) {
            self.remember(here);
        }
    }

    /// Remember where the cursor last was in `path`'s tab, before a jump
    /// the editor is making itself.
    pub(super) fn remember_last_cursor(&mut self, path: &Path) {
        let offset = self
            .tab_for(path)
            .and_then(|ix| self.tabs[ix].last_cursor());
        if let Some(offset) = offset {
            self.remember(Place::Code {
                path: path.to_path_buf(),
                offset,
            });
        }
    }

    fn remember(&mut self, place: Place) {
        History::push(&mut self.history.back, place);
        self.history.forward.clear();
    }

    pub(super) fn go_back(&mut self, _: &GoBack, window: &mut Window, cx: &mut Context<Self>) {
        self.step_history(true, window, cx);
    }

    pub(super) fn go_forward(
        &mut self,
        _: &GoForward,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.step_history(false, window, cx);
    }

    /// A mouse's back or forward button, anywhere in the window.
    pub(super) fn navigation_button(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let MouseButton::Navigate(direction) = event.button else {
            return;
        };
        self.step_history(direction == NavigationDirection::Back, window, cx);
        cx.stop_propagation();
    }

    fn step_history(&mut self, back: bool, window: &mut Window, cx: &mut Context<Self>) {
        let here = self.here(cx);
        let (from, to) = if back {
            (&mut self.history.back, &mut self.history.forward)
        } else {
            (&mut self.history.forward, &mut self.history.back)
        };
        // Places in files since deleted are skipped.
        let Some(place) = std::iter::from_fn(|| from.pop()).find(Place::exists) else {
            return;
        };
        if let Some(here) = here {
            History::push(to, here);
        }
        let (path, offset) = match place {
            Place::Map(level) => return self.map_at(level, window, cx),
            Place::Code { path, offset } => (path, offset),
        };
        self.leave_map(window, cx);
        self.open_file(&path, window, cx);
        if !self.showing(&path) {
            return;
        }
        let text = self.editor().text(cx);
        let mut offset = offset.min(text.len());
        while !text.is_char_boundary(offset) {
            offset -= 1;
        }
        self.editor().select(offset..offset, cx);
        self.editor().focus(window, cx);
        cx.notify();
    }

    #[cfg(test)]
    pub(super) fn history_lengths(&self) -> (usize, usize) {
        (self.history.back.len(), self.history.forward.len())
    }
}
