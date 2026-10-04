//! Filling in a snippet taken from the completion list: its first place is
//! selected, Tab moves to the next and Shift-Tab back, and the last puts the
//! cursor where the snippet ends. Esc, an edit elsewhere or moving away
//! leaves the snippet as it is.
//!
//! GPUI Kit's editor inserts a completion as plain text and doesn't say
//! which one was taken, so the completion provider keeps the snippets it
//! offered ([`Offers`]) and a change that inserted one of them starts it.

use std::cell::RefCell;
use std::ops::Range;
use std::rc::Rc;

use gpui_kit::*;
use jig_editor::EditorHandle;

use super::Workspace;

/// A snippet the user was offered: where it would go, its text, and the
/// places in that text to fill in.
pub(super) struct Offer {
    pub(super) start: usize,
    pub(super) text: String,
    pub(super) stops: Vec<Range<usize>>,
}

pub(super) type Offers = Rc<RefCell<Vec<Offer>>>;

/// A snippet being filled in.
pub(super) struct Session {
    /// Where each place is now, in the buffer.
    stops: Vec<Range<usize>>,
    current: usize,
    /// The buffer's length when last seen, to tell how much an edit added.
    len: usize,
}

impl Session {
    /// Follow an edit that changed the buffer's length to `len` and left the
    /// cursor at `cursor`. False if it wasn't in the current place, which
    /// ends the snippet.
    fn follow(&mut self, len: usize, cursor: usize) -> bool {
        let delta = len as isize - self.len as isize;
        let current = self.stops[self.current].clone();
        let Some(end) = current.end.checked_add_signed(delta) else {
            return false;
        };
        if end < current.start || cursor < current.start || cursor > end {
            return false;
        }
        let mut ix = 0;
        let mut current_ix = self.current;
        self.stops.retain(|stop| {
            let keep = ix == self.current
                // Places inside this one went with what was typed over them.
                || !(stop.start >= current.start
                    && stop.start < current.end
                    && stop.end <= current.end);
            if !keep && ix < self.current {
                current_ix -= 1;
            }
            ix += 1;
            keep
        });
        self.current = current_ix;
        for (ix, stop) in self.stops.iter_mut().enumerate() {
            if ix == self.current {
                stop.end = end;
            } else if stop.start >= current.end {
                stop.start = stop.start.saturating_add_signed(delta);
                stop.end = stop.end.saturating_add_signed(delta);
            }
        }
        self.len = len;
        true
    }
}

impl Workspace {
    /// The tab at `ix` changed: start the snippet it just took from the
    /// completion list, or follow the edit in the one being filled in.
    pub(super) fn snippet_changed(&mut self, ix: usize, cx: &mut Context<Self>) {
        let tab = &mut self.tabs[ix];
        let text = tab.editor.text(cx);
        let cursor = tab.editor.cursor(cx);
        let taken = {
            let mut offers = tab.offers.borrow_mut();
            let taken = offers.iter().position(|offer| {
                cursor == offer.start + offer.text.len()
                    && text.get(offer.start..cursor) == Some(offer.text.as_str())
            });
            taken.map(|taken| {
                let offer = offers.swap_remove(taken);
                offers.clear();
                offer
            })
        };
        if let Some(offer) = taken {
            let stops: Vec<Range<usize>> = offer
                .stops
                .iter()
                .map(|stop| offer.start + stop.start..offer.start + stop.end)
                .collect();
            tab.editor.select(stops[0].clone(), cx);
            // Only somewhere to put the cursor: nothing to fill in.
            tab.snippet = (stops.len() > 1).then_some(Session {
                stops,
                current: 0,
                len: text.len(),
            });
            return;
        }
        if let Some(session) = &mut tab.snippet
            && !session.follow(text.len(), cursor)
        {
            tab.snippet = None;
        }
    }

    /// Tab (or Shift-Tab, going back) in a snippet: select its next place.
    /// False when there's no snippet being filled in here, or the cursor has
    /// left it.
    pub(super) fn snippet_step(&mut self, forward: bool, cx: &mut Context<Self>) -> bool {
        let tab = self.tab_mut();
        let Some(session) = &mut tab.snippet else {
            return false;
        };
        let selection = tab.editor.selection(cx);
        let current = &session.stops[session.current];
        if selection.start < current.start || selection.end > current.end {
            tab.snippet = None;
            return false;
        }
        session.current = if forward {
            session.current + 1
        } else {
            session.current.saturating_sub(1)
        };
        let stop = session.stops[session.current].clone();
        if session.current + 1 == session.stops.len() {
            tab.snippet = None;
        }
        let state = tab.editor.state().clone();
        state.update(cx, |state, cx| state.dismiss_completion_overlay(cx));
        tab.editor.select(stop, cx);
        cx.notify();
        true
    }

    pub(super) fn end_snippet(&mut self) {
        if let Some(tab) = self.tabs.get_mut(self.active) {
            tab.snippet = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Session;

    fn session(stops: &[std::ops::Range<usize>], current: usize, len: usize) -> Session {
        Session {
            stops: stops.to_vec(),
            current,
            len,
        }
    }

    #[test]
    fn typing_over_a_place_moves_the_ones_after_it() {
        // `for item in items {}` with `item` selected, then `x` typed.
        let mut s = session(&[4..8, 12..17, 19..19], 0, 20);
        assert!(s.follow(17, 5));
        assert_eq!(s.stops, [4..5, 9..14, 16..16]);
        // And more typed after it.
        assert!(s.follow(19, 7));
        assert_eq!(s.stops, [4..7, 11..16, 18..18]);
    }

    #[test]
    fn typing_over_a_place_drops_the_places_inside_it() {
        // `Vec<T>` with `T` its own place, all typed over with `u8`.
        let mut s = session(&[0..6, 4..5, 6..6], 0, 6);
        assert!(s.follow(2, 2));
        assert_eq!(s.stops, [0..2, 2..2]);
        assert_eq!(s.current, 0);
    }

    #[test]
    fn an_edit_elsewhere_ends_it() {
        let mut s = session(&[4..8, 12..17, 19..19], 0, 20);
        assert!(!s.follow(21, 30));
    }
}
