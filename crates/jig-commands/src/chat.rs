//! The agent conversation: a floating window at the code with what was asked,
//! what the agent said and did, and a box to reply in. Each reply continues
//! the same agent session, so the agent remembers the turns before.

use std::cell::Cell;
use std::rc::Rc;
use std::time::Instant;

use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::shimmer::ShimmerText;
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{ActiveTheme as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::LiveStep;

/// The conversation's width.
pub const WIDTH: f32 = 460.;

/// Tallest the transcript grows before it scrolls.
const TRANSCRIPT_HEIGHT: f32 = 280.;

/// One line of the conversation.
#[derive(Clone, Debug, PartialEq)]
pub enum ChatEntry {
    /// What the user asked or replied.
    User(String),
    /// The agent's closing words for a turn.
    Agent(String),
    /// An edit the agent proposed, and whether the user took it.
    Edit {
        path: String,
        accepted: bool,
    },
    /// Something that happened to the run, e.g. "Stopped."
    Note(String),
    Error(String),
}

/// What the conversation is waiting on.
#[derive(Clone, Debug, PartialEq)]
pub enum ChatStatus {
    /// The agent is on a turn.
    Working { started: Instant, step: LiveStep },
    /// An edit is in the buffer awaiting accept or reject.
    Reviewing { path: String, removed: String },
    /// The turn is over; the user may reply.
    Waiting,
}

/// What's dragged while the conversation is moved by its header.
pub struct ChatDrag;

/// What's dragged while the conversation is resized by its corner.
pub struct ChatResize;

/// Where something was last drawn, in window coordinates.
pub type ChatBounds = Rc<Cell<Bounds<Pixels>>>;

/// The conversation's size, and where it was last drawn. Kept by its owner
/// from frame to frame.
#[derive(Clone)]
pub struct ChatFrame {
    pub width: Pixels,
    /// Set once the user resizes it; until then the transcript grows with
    /// its content, up to a limit.
    pub transcript_height: Option<Pixels>,
    pub bounds: ChatBounds,
    pub transcript_bounds: ChatBounds,
}

impl Default for ChatFrame {
    fn default() -> Self {
        Self {
            width: px(WIDTH),
            transcript_height: None,
            bounds: Default::default(),
            transcript_bounds: Default::default(),
        }
    }
}

/// The corner grip (two short diagonal strokes).
const GRIP_ICON: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2.5" stroke-linecap="round"><path d="M20 11 11 20"/><path d="M20 17 17 20"/></svg>"#;

type GrabHandler = Rc<dyn Fn(&MouseDownEvent, &mut Window, &mut App)>;

/// The conversation as drawn. The workspace owns the transcript and the
/// reply box; this only lays them out.
#[derive(IntoElement)]
pub struct Conversation {
    label: String,
    entries: Vec<ChatEntry>,
    status: ChatStatus,
    input: Entity<InputState>,
    scroll: ScrollHandle,
    frame: ChatFrame,
    on_grab: Option<GrabHandler>,
    on_resize_grab: Option<GrabHandler>,
}

impl Conversation {
    pub fn new(
        label: String,
        entries: Vec<ChatEntry>,
        status: ChatStatus,
        input: Entity<InputState>,
        scroll: ScrollHandle,
        frame: ChatFrame,
    ) -> Self {
        Self {
            label,
            entries,
            status,
            input,
            scroll,
            frame,
            on_grab: None,
            on_resize_grab: None,
        }
    }

    /// Called when the corner grip is pressed; the workspace then resizes
    /// the window with [`ChatResize`] moves.
    pub fn on_resize_grab(
        mut self,
        handler: impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_resize_grab = Some(Rc::new(handler));
        self
    }

    /// Called when the header is pressed, before a drag may start; the
    /// workspace then moves the window with [`ChatDrag`] moves.
    pub fn on_grab(
        mut self,
        handler: impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_grab = Some(Rc::new(handler));
        self
    }
}

impl RenderOnce for Conversation {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = cx.theme();
        let accent = crate::surface::lane_accent(true, cx);
        let on_grab = self.on_grab;
        // The header is the handle to move the window by.
        let header = h_flex()
            .id("jig-agent-chat-header")
            .debug_selector(|| "agent-chat-header".into())
            .cursor_grab()
            .py_0p5()
            .when_some(on_grab, |this, on_grab| {
                this.on_mouse_down(MouseButton::Left, move |event, window, cx| {
                    on_grab(event, window, cx)
                })
            })
            .on_drag(ChatDrag, |_, _, _, cx| cx.new(|_| EmptyView))
            .gap_2()
            .text_xs()
            .text_color(theme.muted_foreground)
            .child(div().truncate().child(self.label))
            .child(div().flex_1())
            .when_some(
                match &self.status {
                    ChatStatus::Working { started, .. } => Some(started.elapsed().as_secs()),
                    _ => None,
                },
                // The spinner animates every frame, so this stays current.
                |this, elapsed| this.child(format!("{elapsed}s")),
            )
            .child(crate::surface::lane_tag(true, cx));

        let transcript = v_flex()
            .id("jig-agent-transcript")
            .map(|this| match self.frame.transcript_height {
                Some(height) => this.h(height),
                None => this.max_h(px(TRANSCRIPT_HEIGHT)),
            })
            .overflow_y_scroll()
            .track_scroll(&self.scroll)
            .gap_2()
            .children(
                self.entries
                    .into_iter()
                    .map(|entry| render_entry(entry, cx)),
            );
        let transcript_bounds = self.frame.transcript_bounds.clone();
        let transcript = div().relative().child(transcript).child(
            canvas(
                move |drawn, _, _| transcript_bounds.set(drawn),
                |_, _, _, _| {},
            )
            .absolute()
            .top_0()
            .left_0()
            .size_full(),
        );

        let status = match self.status {
            ChatStatus::Working { step, .. } => {
                let step = step.get();
                v_flex()
                    .gap_1p5()
                    .child(
                        h_flex()
                            .gap_2()
                            .text_xs()
                            .child(Spinner::new().color(accent))
                            .child(
                                ShimmerText::new(if step.is_empty() {
                                    "Working…".to_string()
                                } else {
                                    format!("{step}…")
                                })
                                .id("jig-agent-step"),
                            ),
                    )
                    .child(crate::bubble::progress_bar(accent, theme.muted))
                    .child(crate::surface::hint("esc to stop the agent", cx))
            }
            ChatStatus::Reviewing { path, removed } => v_flex()
                .gap_2()
                .child(
                    h_flex()
                        .gap_2()
                        .child("Wants to edit")
                        .child(file_name(path, cx)),
                )
                .when(!removed.trim().is_empty(), |this| {
                    this.child(crate::bubble::removed_lines(&removed, cx))
                })
                .child(crate::surface::hint(
                    "tab or enter to accept · esc to reject",
                    cx,
                )),
            ChatStatus::Waiting => v_flex()
                .gap_1p5()
                .child(
                    div()
                        .id("jig-agent-reply")
                        .debug_selector(|| "agent-reply-box".into())
                        .cursor_text()
                        // The whole box takes the click, not just the text.
                        .on_mouse_down(MouseButton::Left, {
                            let input = self.input.clone();
                            move |_, window, cx| {
                                input.update(cx, |input, cx| input.focus(window, cx));
                            }
                        })
                        .px_2()
                        .py_1()
                        .rounded(px(8.))
                        .border_1()
                        .border_color(accent.opacity(0.35))
                        .bg(theme.foreground.opacity(0.03))
                        .child(
                            Input::new(&self.input)
                                .w_full()
                                .appearance(false)
                                .cleanable(false)
                                .prefix(
                                    crate::surface::lane_icon(true)
                                        .size(px(13.))
                                        .text_color(accent),
                                ),
                        ),
                )
                .child(crate::surface::hint("enter to reply · esc to close", cx)),
        };

        let panel = crate::surface::panel(cx)
            .flex()
            .flex_col()
            .w(self.frame.width)
            .px_3p5()
            .py_2p5()
            .gap_2p5()
            .text_size(px(13.))
            .border_color(accent.opacity(0.45))
            .child(header)
            .child(transcript)
            .child(div().h(px(1.)).bg(theme.foreground.opacity(0.08)))
            .child(status);
        // Remember where it's drawn, outside the panel's slide-in, so a drag
        // starts from there even when it was nudged to fit the window.
        let bounds = self.frame.bounds;
        let grip = div()
            .id("jig-agent-chat-resize")
            .debug_selector(|| "agent-chat-resize".into())
            .absolute()
            .right(px(3.))
            .bottom(px(3.))
            .size(px(14.))
            .cursor_nwse_resize()
            .when_some(self.on_resize_grab, |this, on_grab| {
                this.on_mouse_down(MouseButton::Left, move |event, window, cx| {
                    on_grab(event, window, cx)
                })
            })
            .on_drag(ChatResize, |_, _, _, cx| cx.new(|_| EmptyView))
            .child(
                gpui_kit::component::Icon::default()
                    .data(GRIP_ICON)
                    .size(px(14.))
                    .text_color(theme.muted_foreground.opacity(0.6)),
            );
        div()
            .relative()
            .child(crate::motion::pop_in(panel, "jig-agent-chat"))
            .child(
                canvas(move |drawn, _, _| bounds.set(drawn), |_, _, _, _| {})
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full(),
            )
            .child(grip)
    }
}

fn render_entry(entry: ChatEntry, cx: &App) -> AnyElement {
    let theme = cx.theme();
    match entry {
        // The user's words sit on the right, tinted with the lane.
        ChatEntry::User(text) => h_flex()
            .justify_end()
            .child(
                div()
                    .max_w(relative(0.85))
                    .px_2p5()
                    .py_1p5()
                    .rounded(px(9.))
                    .bg(crate::surface::lane_accent(true, cx).opacity(0.12))
                    .child(text),
            )
            .into_any_element(),
        ChatEntry::Agent(text) => div().child(text).into_any_element(),
        ChatEntry::Edit { path, accepted } => h_flex()
            .gap_1p5()
            .text_xs()
            .text_color(theme.muted_foreground)
            .child(if accepted {
                "Edited"
            } else {
                "Rejected an edit to"
            })
            .child(file_name(path, cx))
            .into_any_element(),
        ChatEntry::Note(text) => div()
            .text_xs()
            .text_color(theme.muted_foreground)
            .child(text)
            .into_any_element(),
        ChatEntry::Error(text) => div()
            .text_color(theme.danger)
            .child(text)
            .into_any_element(),
    }
}

/// A project-relative path, set in the code font.
fn file_name(path: String, cx: &App) -> Div {
    let theme = cx.theme();
    div()
        .px_1p5()
        .rounded(px(4.))
        .bg(theme.foreground.opacity(0.06))
        .font_family(theme.mono_font_family.clone())
        .text_xs()
        .text_color(theme.foreground)
        .child(path)
}
