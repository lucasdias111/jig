//! The agent input (⌘K) and its title-bar button, starting a conversation
//! from it, notes beside the code, and the keys that settle an edit under
//! review.

use std::ops::Range;
use std::time::Duration;

use gpui_kit::component::input::{Enter, Escape, Indent, IndentInline, OutdentInline, Undo};
use gpui_kit::component::{ActiveTheme as _, h_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use jig_commands::{Bubble, CommandPalette, PaletteEvent};
use jig_editor::EditorHandle;

use super::agent::AgentRun;
use super::{OpenCommand, OpenPalette, Workspace};

mod input;
mod review_keys;

/// The selection's button for the agent input (Lucide "sparkles").
pub(super) const SPARKLE_ICON: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M9.937 15.5A2 2 0 0 0 8.5 14.063l-6.135-1.582a.5.5 0 0 1 0-.962L8.5 9.936A2 2 0 0 0 9.937 8.5l1.582-6.135a.5.5 0 0 1 .963 0L14.063 8.5A2 2 0 0 0 15.5 9.937l6.135 1.581a.5.5 0 0 1 0 .964L15.5 14.063a2 2 0 0 0-1.437 1.437l-1.582 6.135a.5.5 0 0 1-.963 0z"/><path d="M20 3v4"/><path d="M22 5h-4"/></svg>"#;

/// How long a note stays up before it fades on its own.
const NOTE_TIMEOUT: Duration = Duration::from_secs(4);
/// Longest error text shown in the bubble.
const MAX_ERROR_CHARS: usize = 180;

/// Background of code the agent was asked about, while it works.
fn working_color(cx: &App) -> Hsla {
    cx.theme().info.opacity(0.16)
}

/// Background of code the agent just wrote, while it awaits review.
pub(super) fn added_color(cx: &App) -> Hsla {
    cx.theme().success.opacity(0.18)
}

/// A conversation with the agent, from the moment it's asked for until
/// it's closed.
pub(super) struct CommandRun {
    pub(super) id: u64,
    pub(super) bubble: Bubble,
    pub(super) anchor: Point<Pixels>,
    /// Set while an edit sits in the buffer awaiting accept or reject.
    pub(super) preview: Option<Preview>,
    /// `None` only until the conversation is set up.
    pub(super) agent: Option<AgentRun>,
    pub(super) _task: Option<Task<()>>,
}

pub(super) struct Preview {
    /// Where the new code is in the buffer.
    pub(super) range: Range<usize>,
}

/// A short note beside the code, fading after a few seconds.
pub(super) struct Note {
    id: u64,
    pub(super) bubble: Bubble,
    pub(super) anchor: Point<Pixels>,
    _task: Task<()>,
}

/// What a conversation is started with, from ⌘K, Quick Fix or a task.
pub(super) struct Ask {
    /// How the conversation is titled, e.g. "Fix"; `None` uses what was
    /// asked.
    pub(super) name: Option<String>,
    pub(super) instruction: String,
    /// Send the problems the language server reports on the lines asked
    /// about.
    pub(super) diagnostics: bool,
    /// The planned task this is about, as context.
    pub(super) task: Option<String>,
}

impl Ask {
    pub(super) fn new(instruction: impl Into<String>) -> Self {
        Self {
            name: None,
            instruction: instruction.into(),
            diagnostics: false,
            task: None,
        }
    }
}

impl Workspace {
    /// Where floating UI opens: just below the cursor or selection.
    pub(super) fn floating_anchor(&self, cx: &App) -> Point<Pixels> {
        self.editor()
            .anchor_point(cx)
            .map(|point| point + gpui_kit::point(px(-8.), px(4.)))
            // The cursor is scrolled out of view: open near the top instead.
            .unwrap_or(gpui_kit::point(px(48.), px(48.)))
    }

    /// Where a floating panel `width` wide, growing to `height`, opens:
    /// centred in the window's right half and top to bottom, pulled in from
    /// the edge (or a docked conversation) in a narrow window, and below the
    /// title bar in a short one.
    pub(super) fn right_centre(
        &self,
        width: Pixels,
        height: Pixels,
        window: &Window,
    ) -> Point<Pixels> {
        const MARGIN: f32 = 8.;
        let viewport = window.viewport_size();
        let edge = viewport.width - self.docked_chat_width();
        let x = (viewport.width * 0.75 - width / 2.)
            .min(edge - width - px(MARGIN))
            .max(px(MARGIN));
        let y = ((viewport.height - height) / 2.).max(px(super::TITLE_BAR_HEIGHT + 8.));
        gpui_kit::point(x, y)
    }

    /// The agent input or a form has the keyboard.
    pub(super) fn modal_open(&self, cx: &App) -> bool {
        self.palette.is_some()
            || self.task_form.read(cx).is_open()
            || self.quick_open.read(cx).is_open()
            || self.find_in_files.read(cx).is_open()
    }

    /// Close the agent input and any form, e.g. once the keyboard has gone
    /// back to the code without them.
    fn close_modals(&mut self, cx: &mut Context<Self>) {
        self.palette = None;
        self.task_form.update(cx, |sheet, cx| sheet.close(cx));
        self.close_quick_open(cx);
        self.close_find_in_files(cx);
    }

    /// Show a short note at the cursor that fades after a few seconds.
    pub(super) fn show_note(
        &mut self,
        message: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.next_run_id += 1;
        let id = self.next_run_id;
        self.note = Some(Note {
            id,
            bubble: Bubble::Message(message),
            anchor: self.floating_anchor(cx),
            _task: cx.spawn_in(window, async move |this, cx| {
                cx.background_executor().timer(NOTE_TIMEOUT).await;
                this.update(cx, |this, cx| {
                    if this.note.as_ref().is_some_and(|note| note.id == id) {
                        this.note = None;
                        cx.notify();
                    }
                })
                .ok();
            }),
        });
        cx.notify();
    }

    /// Any edit makes a note stale.
    pub(super) fn on_buffer_changed(&mut self, cx: &mut Context<Self>) {
        if self.note.take().is_some() {
            cx.notify();
        }
    }
}

/// The line containing `offset`, without its trailing newline.
pub(super) fn line_at(text: &str, offset: usize) -> Range<usize> {
    let offset = offset.min(text.len());
    let start = text[..offset].rfind('\n').map_or(0, |at| at + 1);
    let end = text[offset..]
        .find('\n')
        .map_or(text.len(), |at| offset + at);
    start..end
}

pub(super) fn shorten(error: &str) -> String {
    let error = error.trim();
    if error.chars().count() <= MAX_ERROR_CHARS {
        error.to_string()
    } else {
        format!(
            "{}…",
            error.chars().take(MAX_ERROR_CHARS).collect::<String>()
        )
    }
}
