//! The agent conversation: a floating window at the code with what was asked,
//! what the agent said and did, and a box to reply in. Each reply continues
//! the same agent session, so the agent remembers the turns before. It can
//! be pinned to the window's right edge, and its header opens the earlier
//! conversations.

use std::cell::Cell;
use std::rc::Rc;
use std::time::Instant;

use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::shimmer::ShimmerText;
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::text::{TextView, TextViewStyle};
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
    /// Something the agent did besides editing, e.g. "Ran `cargo test`".
    Did(String),
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
    /// The agent wants to do something else that needs the user's go-ahead,
    /// e.g. run a command.
    Asking { title: String, detail: String },
    /// The agent asked a question; the user picks an option or answers in
    /// the reply box.
    Question {
        question: String,
        options: Vec<String>,
        /// Which of how many questions this is, when it asked several.
        position: (usize, usize),
    },
    /// An earlier conversation is being fetched.
    Loading,
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
    /// Docked to the window's right edge, full height, instead of floating
    /// at the code.
    pub pinned: bool,
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
            pinned: false,
            transcript_height: None,
            bounds: Default::default(),
            transcript_bounds: Default::default(),
        }
    }
}

/// Pin to the side (Lucide "panel-right").
const PIN_ICON: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect width="18" height="18" x="3" y="3" rx="2"/><path d="M15 3v18"/></svg>"#;

/// Earlier conversations (Lucide "history").
const HISTORY_ICON: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M3 12a9 9 0 1 0 9-9 9.75 9.75 0 0 0-6.74 2.74L3 8"/><path d="M3 3v5h5"/><path d="M12 7v5l4 2"/></svg>"#;

/// The corner grip (two short diagonal strokes).
const GRIP_ICON: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2.5" stroke-linecap="round"><path d="M20 11 11 20"/><path d="M20 17 17 20"/></svg>"#;

type GrabHandler = Rc<dyn Fn(&MouseDownEvent, &mut Window, &mut App)>;
type ClickHandler = Rc<dyn Fn(&ClickEvent, &mut Window, &mut App)>;
type OptionHandler = Rc<dyn Fn(&usize, &mut Window, &mut App)>;

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
    on_pin: Option<ClickHandler>,
    on_history: Option<ClickHandler>,
    on_option: Option<OptionHandler>,
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
            on_pin: None,
            on_history: None,
            on_option: None,
        }
    }

    /// Called by the header's pin button.
    pub fn on_pin(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_pin = Some(Rc::new(handler));
        self
    }

    /// Called by the header's button for the earlier conversations.
    pub fn on_history(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_history = Some(Rc::new(handler));
        self
    }

    /// Called with the option clicked when the agent asks a question.
    pub fn on_option(mut self, handler: impl Fn(&usize, &mut Window, &mut App) + 'static) -> Self {
        self.on_option = Some(Rc::new(handler));
        self
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
        let pinned = self.frame.pinned;
        let on_grab = self.on_grab.filter(|_| !pinned);
        let header_button =
            |id: &'static str, icon: &'static [u8], active: bool, handler: Option<ClickHandler>| {
                let theme = cx.theme();
                div()
                    .id(id)
                    .debug_selector(move || id.into())
                    .flex_none()
                    .p_0p5()
                    .rounded(px(4.))
                    .cursor_pointer()
                    .hover(|this| this.bg(theme.foreground.opacity(0.08)))
                    .when(active, |this| this.bg(accent.opacity(0.16)))
                    // Not a drag of the header.
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .when_some(handler, |this, handler| {
                        this.on_click(move |event, window, cx| handler(event, window, cx))
                    })
                    .child(
                        gpui_kit::component::Icon::default()
                            .data(icon)
                            .size(px(13.))
                            .text_color(if active {
                                accent
                            } else {
                                theme.muted_foreground
                            }),
                    )
            };
        let history = header_button("agent-chat-history", HISTORY_ICON, false, self.on_history);
        let pin = header_button("agent-chat-pin", PIN_ICON, pinned, self.on_pin);
        // The header is the handle to move the window by.
        let header = h_flex()
            .id("jig-agent-chat-header")
            .debug_selector(|| "agent-chat-header".into())
            .when(!pinned, |this| this.cursor_grab())
            .py_0p5()
            .when_some(on_grab, |this, on_grab| {
                this.on_mouse_down(MouseButton::Left, move |event, window, cx| {
                    on_grab(event, window, cx)
                })
                .on_drag(ChatDrag, |_, _, _, cx| cx.new(|_| EmptyView))
            })
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
            .child(crate::surface::lane_tag(true, cx))
            .child(history)
            .child(pin);

        let transcript = v_flex()
            .id("jig-agent-transcript")
            .map(|this| match (pinned, self.frame.transcript_height) {
                (true, _) => this.size_full(),
                (false, Some(height)) => this.h(height),
                (false, None) => this.max_h(px(TRANSCRIPT_HEIGHT)),
            })
            .overflow_y_scroll()
            .track_scroll(&self.scroll)
            .gap_2()
            .children(
                self.entries
                    .into_iter()
                    .enumerate()
                    .map(|(index, entry)| render_entry(index, entry, cx)),
            );
        let transcript_bounds = self.frame.transcript_bounds.clone();
        let transcript = div()
            .relative()
            .when(pinned, |this| this.flex_1().min_h_0())
            .child(transcript)
            .child(
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
            ChatStatus::Asking { title, detail } => v_flex()
                .gap_2()
                .child(
                    div()
                        .font_weight(FontWeight::MEDIUM)
                        .child(format!("{title}?")),
                )
                .when(!detail.trim().is_empty(), |this| {
                    this.child(
                        div()
                            .px_2()
                            .py_1()
                            .rounded(px(6.))
                            .bg(theme.foreground.opacity(0.05))
                            .font_family(theme.mono_font_family.clone())
                            .text_xs()
                            .child(detail),
                    )
                })
                .child(crate::surface::hint(
                    "tab or enter to allow · esc to deny",
                    cx,
                )),
            ChatStatus::Question {
                question,
                options,
                position: (at, of),
            } => {
                let on_option = self.on_option.clone();
                v_flex()
                    .gap_2()
                    .child(
                        div()
                            .font_weight(FontWeight::MEDIUM)
                            .when(of > 1, |this| this.child(format!("{at}/{of} ")))
                            .child(question),
                    )
                    .child(h_flex().flex_wrap().gap_1p5().children(
                        options.into_iter().enumerate().map(|(index, option)| {
                            let on_option = on_option.clone();
                            div()
                                .id(("jig-agent-option", index))
                                .px_2()
                                .py_0p5()
                                .rounded(px(6.))
                                .border_1()
                                .border_color(accent.opacity(0.35))
                                .cursor_pointer()
                                .hover(|this| this.bg(accent.opacity(0.12)))
                                .text_xs()
                                .child(format!("{} {option}", index + 1))
                                .when_some(on_option, |this, on_option| {
                                    this.on_click(move |_, window, cx| {
                                        on_option(&index, window, cx)
                                    })
                                })
                        }),
                    ))
                    .child(reply_box(&self.input, accent, cx))
                    .child(crate::surface::hint(
                        "a number or your own answer · enter to answer · esc to decline",
                        cx,
                    ))
            }
            ChatStatus::Loading => h_flex()
                .gap_2()
                .text_xs()
                .child(Spinner::new().color(accent))
                .child("Loading the conversation…"),
            ChatStatus::Waiting => v_flex()
                .gap_1p5()
                .child(reply_box(&self.input, accent, cx))
                .child(crate::surface::hint("enter to reply · esc to close", cx)),
        };
        let panel = crate::surface::panel(cx)
            .flex()
            .flex_col()
            .w(self.frame.width)
            // Pinned, it fills the height its owner gives it.
            .when(pinned, |this| this.h_full())
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
            .when(pinned, |this| this.h_full())
            .child(crate::motion::pop_in(panel, "jig-agent-chat"))
            .child(
                canvas(move |drawn, _, _| bounds.set(drawn), |_, _, _, _| {})
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full(),
            )
            .when(!pinned, |this| this.child(grip))
    }
}

/// The box the user replies or answers in. The whole box takes the click,
/// not just the text.
fn reply_box(input: &Entity<InputState>, accent: Hsla, cx: &App) -> Stateful<Div> {
    let theme = cx.theme();
    div()
        .id("jig-agent-reply")
        .debug_selector(|| "agent-reply-box".into())
        .cursor_text()
        .on_mouse_down(MouseButton::Left, {
            let input = input.clone();
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
            Input::new(input)
                .w_full()
                .appearance(false)
                .cleanable(false)
                .prefix(
                    crate::surface::lane_icon(true)
                        .size(px(13.))
                        .text_color(accent),
                ),
        )
}

fn render_entry(index: usize, entry: ChatEntry, cx: &App) -> AnyElement {
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
        // The agent may answer at length, in Markdown.
        ChatEntry::Agent(text) => {
            let code = StyleRefinement::default()
                .font_family(theme.mono_font_family.clone())
                .text_size(px(12.));
            TextView::markdown(("jig-agent-text", index), text)
                .style(
                    TextViewStyle::default()
                        .paragraph_gap(rems(0.5))
                        .code_block(code),
                )
                .selectable(true)
                .into_any_element()
        }
        ChatEntry::Did(text) => div()
            .text_xs()
            .text_color(theme.muted_foreground)
            .child(text)
            .into_any_element(),
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
