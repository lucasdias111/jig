//! The agent conversation: a floating window at the code with what was asked,
//! what the agent said and did, and a box to reply in. Each reply continues
//! the same agent session, so the agent remembers the turns before. It can
//! be pinned to the window's right edge, and its header opens the earlier
//! conversations.

use std::cell::Cell;
use std::rc::Rc;
use std::time::Instant;

use gpui_kit::component::input::{Enter, Textarea, TextareaState};
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::shimmer::ShimmerText;
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::text::{TextView, TextViewStyle};
use gpui_kit::component::{ActiveTheme as _, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::LiveStep;

/// The conversation's width.
pub const WIDTH: f32 = 460.;

/// Tallest the transcript grows before it scrolls.
const TRANSCRIPT_HEIGHT: f32 = 280.;

/// Tallest a request's command (or other detail) grows before it scrolls.
const REQUEST_DETAIL_HEIGHT: f32 = 140.;

/// One line of the conversation.
#[derive(Clone, Debug, PartialEq)]
pub enum ChatEntry {
    /// What the user asked or replied.
    User(String),
    /// The agent's closing words for a turn.
    Agent(String),
    /// An edit the agent proposed, and whether the user took it. `diff`
    /// is what it changed, opened from its row; empty when not known, as
    /// for an earlier conversation picked up again.
    Edit {
        path: String,
        accepted: bool,
        diff: Vec<DiffLine>,
    },
    /// Something the agent did besides editing, e.g. "Ran `cargo test`".
    Did(String),
    /// A subagent it started: what it was given to do, each thing it did,
    /// and once it's done, its report back. Opened from its row.
    Subagent {
        id: String,
        description: String,
        calls: Vec<String>,
        report: Option<String>,
    },
    /// Something the agent drew on the Map: its summary, whether it shows
    /// there and whether it's pinned. `stepped` drawings can be toured;
    /// `settled` says what became of a proposal ("Kept", "Dismissed").
    Drawing {
        id: usize,
        summary: String,
        shown: bool,
        pinned: bool,
        stepped: bool,
        settled: Option<String>,
    },
    /// Something that happened to the run, e.g. "Stopped."
    Note(String),
    Error(String),
}

/// What a drawing's row in the transcript was asked to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DrawingAction {
    /// Show it on the Map, or hide it.
    Toggle,
    Pin,
    Tour,
}

/// One line of an edit's diff.
#[derive(Clone, Debug, PartialEq)]
pub enum DiffLine {
    /// Unchanged, shown around a change.
    Same(String),
    Removed(String),
    Added(String),
    /// Unchanged lines left out between two changes.
    Gap,
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
    Asking {
        title: String,
        detail: String,
        /// The label of the button that allows it always.
        always: String,
    },
    /// The agent proposes saving its plan as a task: the user saves it,
    /// turns it down, or says in the reply box what to change.
    SaveTask { title: String, todos: Vec<String> },
    /// The agent asked a question; the user picks an option or answers in
    /// the reply box.
    Question {
        question: String,
        options: Vec<String>,
        /// Which of how many questions this is, when it asked several.
        position: (usize, usize),
    },
    /// The agent wants the user to sign in; each option is a way to.
    SignIn {
        options: Vec<String>,
        /// A terminal was opened for the login.
        opened: bool,
    },
    /// An earlier conversation is being fetched.
    Loading,
    /// The turn is over; the user may reply.
    Waiting,
}

/// The conversation's model, shown in its header and picked from there.
#[derive(Clone, Debug, Default)]
pub struct ChatModels {
    /// Each model's label, e.g. "Opus 5.5".
    pub options: Vec<String>,
    pub current: Option<usize>,
    /// The list is open.
    pub open: bool,
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

/// About how tall the header and reply box make the conversation, over its
/// transcript.
const CHROME_HEIGHT: f32 = 120.;

impl ChatFrame {
    /// How tall it floats once its transcript is full, for placing it before
    /// it has grown.
    pub fn full_height(&self) -> Pixels {
        self.transcript_height.unwrap_or(px(TRANSCRIPT_HEIGHT)) + px(CHROME_HEIGHT)
    }
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

/// Lucide "chevron-right".
const CHEVRON: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="m9 18 6-6-6-6"/></svg>"#;
/// Lucide "chevron-down".
const CHEVRON_DOWN: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="m6 9 6 6 6-6"/></svg>"#;

/// The corner grip (two short diagonal strokes).
const GRIP_ICON: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2.5" stroke-linecap="round"><path d="M20 11 11 20"/><path d="M20 17 17 20"/></svg>"#;

type GrabHandler = Rc<dyn Fn(&MouseDownEvent, &mut Window, &mut App)>;
type SubmitHandler = Rc<dyn Fn(&Enter, &mut Window, &mut App)>;
type ClickHandler = Rc<dyn Fn(&ClickEvent, &mut Window, &mut App)>;
type OptionHandler = Rc<dyn Fn(&usize, &mut Window, &mut App)>;
type DrawingHandler = Rc<dyn Fn(&(usize, DrawingAction), &mut Window, &mut App)>;
/// Called with a link clicked in the agent's words, instead of opening it.
pub type LinkHandler = std::sync::Arc<dyn Fn(&SharedString, &mut Window, &mut App) + Send + Sync>;

/// The conversation as drawn. The workspace owns the transcript and the
/// reply box; this only lays them out.
#[derive(IntoElement)]
pub struct Conversation {
    label: String,
    entries: Vec<ChatEntry>,
    status: ChatStatus,
    input: Entity<TextareaState>,
    scroll: ScrollHandle,
    frame: ChatFrame,
    full_page: bool,
    on_submit: Option<SubmitHandler>,
    on_grab: Option<GrabHandler>,
    on_resize_grab: Option<GrabHandler>,
    on_pin: Option<ClickHandler>,
    on_history: Option<ClickHandler>,
    on_option: Option<OptionHandler>,
    on_decide: Option<OptionHandler>,
    on_show_edit: Option<ClickHandler>,
    models: Option<ChatModels>,
    on_models: Option<ClickHandler>,
    on_model: Option<OptionHandler>,
    on_drawing: Option<DrawingHandler>,
    on_link: Option<LinkHandler>,
}

impl Conversation {
    pub fn new(
        label: String,
        entries: Vec<ChatEntry>,
        status: ChatStatus,
        input: Entity<TextareaState>,
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
            full_page: false,
            on_submit: None,
            on_grab: None,
            on_resize_grab: None,
            on_pin: None,
            on_history: None,
            on_option: None,
            on_decide: None,
            on_show_edit: None,
            models: None,
            on_models: None,
            on_model: None,
            on_drawing: None,
            on_link: None,
        }
    }

    /// Send a message with Enter; Shift-Enter keeps writing on a new line.
    pub fn on_submit(mut self, handler: impl Fn(&Enter, &mut Window, &mut App) + 'static) -> Self {
        self.on_submit = Some(Rc::new(handler));
        self
    }

    /// Fill the workspace with the same transcript and approval controls.
    pub fn full_page(mut self, full_page: bool) -> Self {
        self.full_page = full_page;
        self
    }

    /// Called when a drawing's row is asked to show or hide it, pin it or
    /// tour it.
    pub fn on_drawing(
        mut self,
        handler: impl Fn(&(usize, DrawingAction), &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_drawing = Some(Rc::new(handler));
        self
    }

    /// Called with a link clicked in the agent's words.
    pub fn on_link(mut self, handler: LinkHandler) -> Self {
        self.on_link = Some(handler);
        self
    }

    /// The models it can run on, picked from its header: `on_toggle` opens
    /// and closes the list, `on_pick` gets the one picked.
    pub fn models(
        mut self,
        models: ChatModels,
        on_toggle: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
        on_pick: impl Fn(&usize, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.models = Some(models);
        self.on_models = Some(Rc::new(on_toggle));
        self.on_model = Some(Rc::new(on_pick));
        self
    }

    /// Called when the edit under review is clicked, to show its file.
    pub fn on_show_edit(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_show_edit = Some(Rc::new(handler));
        self
    }

    /// Called with the button pressed on a request: 0 allows it, 1 allows
    /// it always, 2 denies it.
    pub fn on_decide(mut self, handler: impl Fn(&usize, &mut Window, &mut App) + 'static) -> Self {
        self.on_decide = Some(Rc::new(handler));
        self
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
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let empty = self.entries.is_empty();
        let working = matches!(&self.status, ChatStatus::Working { .. });
        let full_page = self.full_page;
        let entries = transcript_rows(
            self.entries,
            self.on_drawing.clone(),
            self.on_link.clone(),
            window,
            cx,
        );
        let theme = cx.theme();
        let accent = crate::surface::agent_accent(cx);
        let pinned = self.frame.pinned || full_page;
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
            .children(self.models.as_ref().and_then(|models| {
                let label = models.options.get(models.current?)?.clone();
                let on_toggle = self.on_models.clone();
                let theme = cx.theme();
                Some(
                    h_flex()
                        .id("agent-chat-model")
                        .debug_selector(|| "agent-chat-model".into())
                        .flex_none()
                        .max_w(px(160.))
                        .gap_0p5()
                        .px_1()
                        .rounded(px(4.))
                        .cursor_pointer()
                        .hover(|this| this.bg(theme.foreground.opacity(0.08)))
                        .when(models.open, |this| {
                            this.bg(accent.opacity(0.16)).text_color(accent)
                        })
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .when_some(on_toggle, |this, on_toggle| {
                            this.on_click(move |event, window, cx| on_toggle(event, window, cx))
                        })
                        .child(div().truncate().child(label))
                        .child("▾"),
                )
            }))
            .child(crate::surface::agent_tag(cx))
            .child(history)
            .when(!full_page, |this| this.child(pin));
        // The models, under the header while their list is open.
        let model_list = self.models.filter(|models| models.open).map(|models| {
            let on_pick = self.on_model.clone();
            v_flex()
                .gap_0p5()
                .p_1()
                .rounded(px(8.))
                .bg(theme.foreground.opacity(0.04))
                .children(
                    models
                        .options
                        .into_iter()
                        .enumerate()
                        .map(|(index, option)| {
                            let on_pick = on_pick.clone();
                            let current = models.current == Some(index);
                            h_flex()
                                .id(("jig-agent-model", index))
                                .debug_selector(move || format!("agent-model-{index}"))
                                .px_2()
                                .py_0p5()
                                .rounded(px(5.))
                                .justify_between()
                                .cursor_pointer()
                                .text_xs()
                                .hover(|this| this.bg(accent.opacity(0.12)))
                                .when(current, |this| this.text_color(accent))
                                .child(option)
                                .when(current, |this| this.child("in use"))
                                .when_some(on_pick, |this, on_pick| {
                                    this.on_click(move |_, window, cx| on_pick(&index, window, cx))
                                })
                        }),
                )
        });

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
            .when(full_page && empty, |this| {
                this.child(
                    v_flex()
                        .flex_1()
                        .justify_center()
                        .items_center()
                        .gap_3()
                        .py_8()
                        .child(
                            crate::surface::agent_icon()
                                .size(px(32.))
                                .text_color(accent),
                        )
                        .child(
                            div()
                                .text_size(px(22.))
                                .child("What would you like to build?"),
                        )
                        .child(
                            div()
                                .text_color(theme.muted_foreground)
                                .child("Ask about your project, plan a change, or start coding."),
                        ),
                )
            })
            .children(entries);
        let more_below = follow_the_end(&self.scroll);
        let transcript_bounds = self.frame.transcript_bounds.clone();
        let transcript = div()
            .relative()
            .when(pinned, |this| this.flex_1().min_h_0())
            .child(transcript)
            .vertical_scrollbar(&self.scroll)
            .when(more_below, |this| {
                this.child(more_below_button(&self.scroll, accent, cx))
            })
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
                        .id("jig-agent-edit-file")
                        .debug_selector(|| "agent-edit-file".into())
                        .gap_2()
                        .cursor_pointer()
                        .child("Wants to edit")
                        .child(file_name(path, cx))
                        .when_some(self.on_show_edit.clone(), |this, on_show| {
                            this.on_click(move |event, window, cx| on_show(event, window, cx))
                        }),
                )
                .when(!removed.trim().is_empty(), |this| {
                    this.child(crate::bubble::removed_lines(&removed, cx))
                })
                .child(
                    h_flex()
                        .flex_wrap()
                        .gap_1p5()
                        .child(decide_button(0, "Accept", "↩", true, &self.on_decide, cx))
                        .child(decide_button(
                            1,
                            "Accept all",
                            ALWAYS_KEY,
                            false,
                            &self.on_decide,
                            cx,
                        ))
                        .child(decide_button(
                            2,
                            "Reject",
                            "esc",
                            false,
                            &self.on_decide,
                            cx,
                        )),
                ),
            ChatStatus::Asking {
                title,
                detail,
                always,
            } => v_flex()
                .gap_2()
                .child(
                    div()
                        .font_weight(FontWeight::MEDIUM)
                        .child(format!("{title}?")),
                )
                .when(!detail.trim().is_empty(), |this| {
                    // A long command scrolls in its box, so the buttons
                    // below it stay in reach.
                    this.child(
                        div()
                            .id("jig-agent-request-detail")
                            .debug_selector(|| "agent-request-detail".into())
                            .max_h(px(REQUEST_DETAIL_HEIGHT))
                            .overflow_y_scroll()
                            .px_2()
                            .py_1()
                            .rounded(px(6.))
                            .bg(theme.foreground.opacity(0.05))
                            .font_family(theme.mono_font_family.clone())
                            .text_xs()
                            .child(detail),
                    )
                })
                .child(
                    h_flex()
                        .flex_wrap()
                        .gap_1p5()
                        .child(decide_button(0, "Allow", "↩", true, &self.on_decide, cx))
                        .child(decide_button(
                            1,
                            always,
                            ALWAYS_KEY,
                            false,
                            &self.on_decide,
                            cx,
                        ))
                        .child(decide_button(2, "Deny", "esc", false, &self.on_decide, cx)),
                ),
            ChatStatus::SaveTask { title, todos } => v_flex()
                .gap_2()
                .child(
                    div()
                        .font_weight(FontWeight::MEDIUM)
                        .child("Save this plan as a task?"),
                )
                .child(
                    v_flex()
                        .id("jig-agent-task-draft")
                        .debug_selector(|| "agent-task-draft".into())
                        .max_h(px(REQUEST_DETAIL_HEIGHT))
                        .overflow_y_scroll()
                        .gap_0p5()
                        .px_2()
                        .py_1()
                        .rounded(px(6.))
                        .bg(theme.foreground.opacity(0.05))
                        .text_xs()
                        .when(!title.is_empty(), |this| {
                            this.child(div().font_weight(FontWeight::MEDIUM).child(title))
                        })
                        .children(todos.into_iter().enumerate().map(|(index, todo)| {
                            h_flex()
                                .items_start()
                                .gap_1p5()
                                .child(
                                    div()
                                        .text_color(theme.muted_foreground)
                                        .child(format!("{}.", index + 1)),
                                )
                                .child(div().flex_1().min_w_0().child(todo))
                        })),
                )
                .child(
                    h_flex()
                        .flex_wrap()
                        .gap_1p5()
                        .child(decide_button(0, "Save", "↩", true, &self.on_decide, cx))
                        .child(decide_button(
                            2,
                            "Don't save",
                            "esc",
                            false,
                            &self.on_decide,
                            cx,
                        )),
                )
                .child(reply_box(&self.input, self.on_submit.clone(), accent, cx))
                .child(crate::surface::hint(
                    "enter to save, or say what to change · esc not to save",
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
                    .child(reply_box(&self.input, self.on_submit.clone(), accent, cx))
                    .child(crate::surface::hint(
                        "a number or your own answer · enter to answer · esc to decline",
                        cx,
                    ))
            }
            ChatStatus::SignIn { options, opened } => {
                let on_option = self.on_option.clone();
                v_flex()
                    .gap_2()
                    .child(
                        v_flex()
                            .gap_1()
                            .children(options.into_iter().enumerate().map(|(index, option)| {
                                let on_option = on_option.clone();
                                div()
                                    .id(("jig-agent-sign-in", index))
                                    .px_2()
                                    .py_1()
                                    .rounded(px(6.))
                                    .border_1()
                                    .border_color(accent.opacity(0.35))
                                    .cursor_pointer()
                                    .hover(|this| this.bg(accent.opacity(0.12)))
                                    .text_xs()
                                    .child(option)
                                    .when_some(on_option, |this, on_option| {
                                        this.on_click(move |_, window, cx| {
                                            on_option(&index, window, cx)
                                        })
                                    })
                            })),
                    )
                    .when(opened, |this| {
                        this.child(reply_box(&self.input, self.on_submit.clone(), accent, cx))
                    })
                    .child(crate::surface::hint(
                        if opened {
                            "finish in the terminal, then enter to try again"
                        } else {
                            "click one to sign in · esc to close"
                        },
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
                .child(reply_box(&self.input, self.on_submit.clone(), accent, cx))
                .child(crate::surface::hint(
                    if full_page {
                        "enter to send · shift-enter for a new line · esc to stop"
                    } else {
                        "enter to reply · shift-enter for a new line · esc to close"
                    },
                    cx,
                )),
        };
        let panel = if full_page {
            div().text_color(theme.foreground)
        } else {
            crate::surface::panel(cx)
        }
        .flex()
        .flex_col()
        .map(|this| {
            if full_page {
                this.w_full()
            } else {
                this.w(self.frame.width)
            }
        })
        // Pinned, it fills the height its owner gives it.
        .when(pinned, |this| this.h_full())
        .px_3p5()
        .py_2p5()
        .gap_2p5()
        .text_size(px(13.))
        .border_color(accent.opacity(0.45))
        .child(header)
        .children(model_list)
        .child(transcript)
        .child(div().h(px(1.)).bg(theme.foreground.opacity(0.08)))
        .child(status)
        .when(full_page && working, |this| {
            this.child(reply_box(&self.input, self.on_submit.clone(), accent, cx))
        });
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
            .when(full_page, |this| this.w_full())
            .when(pinned, |this| this.h_full())
            .child(if full_page {
                panel.into_any_element()
            } else {
                crate::motion::pop_in(panel, "jig-agent-chat").into_any_element()
            })
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

/// The key for "always": allowing a kind of request, or accepting every
/// edit, for the rest of the conversation.
const ALWAYS_KEY: &str = if cfg!(target_os = "macos") {
    "⌘↩"
} else {
    "ctrl ↩"
};

/// One of a request's buttons, labelled with its key: 0 says yes, 1 yes
/// for the rest of the conversation, 2 no.
fn decide_button(
    index: usize,
    label: impl Into<SharedString>,
    key: &'static str,
    primary: bool,
    on_decide: &Option<OptionHandler>,
    cx: &App,
) -> impl IntoElement {
    let theme = cx.theme();
    let accent = crate::surface::agent_accent(cx);
    let on_decide = on_decide.clone();
    h_flex()
        .id(("jig-agent-decide", index))
        .debug_selector(move || format!("agent-decide-{index}"))
        .gap_1p5()
        .px_2()
        .py_0p5()
        .rounded(px(6.))
        .border_1()
        .border_color(accent.opacity(0.35))
        .when(primary, |this| this.bg(accent.opacity(0.16)))
        .cursor_pointer()
        .hover(|this| this.bg(accent.opacity(0.22)))
        .text_xs()
        .child(div().truncate().child(label.into()))
        .child(div().text_color(theme.muted_foreground).child(key))
        .when_some(on_decide, |this, on_decide| {
            this.on_click(move |_, window, cx| on_decide(&index, window, cx))
        })
}

/// Keep the transcript at its end while it's scrolled there, so a reply
/// shows its last words: Markdown is laid out a frame after it arrives, so
/// a single scroll to the bottom stops short of a long reply. Scrolled up,
/// it stays put. Returns whether there's more below what shows.
fn follow_the_end(scroll: &ScrollHandle) -> bool {
    let max = scroll.max_offset().y;
    let below = max + scroll.offset().y;
    if below <= px(4.) {
        scroll.scroll_to_bottom();
        false
    } else {
        true
    }
}

/// Shown while the transcript is scrolled up from its end; jumps there.
fn more_below_button(scroll: &ScrollHandle, accent: Hsla, cx: &App) -> impl IntoElement {
    let theme = cx.theme();
    let scroll = scroll.clone();
    h_flex()
        .absolute()
        .bottom_1()
        .left_0()
        .right_0()
        .justify_center()
        .child(
            h_flex()
                .id("jig-agent-more-below")
                .debug_selector(|| "agent-more-below".into())
                .gap_1()
                .px_2()
                .py_0p5()
                .rounded_full()
                .border_1()
                .border_color(theme.border)
                .bg(theme.popover)
                .shadow_sm()
                .text_xs()
                .text_color(accent)
                .cursor_pointer()
                .child("More below ↓")
                .on_click(move |_, window, _| {
                    scroll.scroll_to_bottom();
                    window.refresh();
                }),
        )
}

/// The shared message composer, also used before a conversation is selected.
pub fn message_input(
    input: &Entity<TextareaState>,
    on_submit: impl Fn(&Enter, &mut Window, &mut App) + 'static,
    cx: &App,
) -> Stateful<Div> {
    reply_box(
        input,
        Some(Rc::new(on_submit)),
        crate::surface::agent_accent(cx),
        cx,
    )
}

/// The box the user replies or answers in. The whole box takes the click,
/// not just the text.
fn reply_box(
    input: &Entity<TextareaState>,
    on_submit: Option<SubmitHandler>,
    accent: Hsla,
    cx: &App,
) -> Stateful<Div> {
    let theme = cx.theme();
    div()
        .id("jig-agent-reply")
        .debug_selector(|| "agent-reply-box".into())
        .when_some(on_submit, |this, submit| {
            this.capture_action(move |action: &Enter, window, cx| {
                if !action.shift {
                    cx.stop_propagation();
                    submit(action, window, cx);
                }
            })
        })
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
        .child(Textarea::new(input).w_full().appearance(false))
}

/// The transcript's rows. A run of two or more tool calls folds into one
/// "Used 5 tools" row that opens to list them, so the agent's words aren't
/// lost among its `grep`s.
fn transcript_rows(
    entries: Vec<ChatEntry>,
    on_drawing: Option<DrawingHandler>,
    on_link: Option<LinkHandler>,
    window: &mut Window,
    cx: &mut App,
) -> Vec<AnyElement> {
    let mut rows = Vec::new();
    let mut entries = entries.into_iter().enumerate().peekable();
    while let Some((index, entry)) = entries.next() {
        let first = match entry {
            ChatEntry::Did(first) => first,
            ChatEntry::Edit {
                path,
                accepted,
                diff,
            } if !diff.is_empty() => {
                rows.push(edit_with_diff(index, path, accepted, diff, window, cx));
                continue;
            }
            ChatEntry::Subagent {
                description,
                calls,
                report,
                ..
            } => {
                rows.push(subagent(
                    index,
                    description,
                    calls,
                    report,
                    on_link.clone(),
                    window,
                    cx,
                ));
                continue;
            }
            ChatEntry::Drawing {
                id,
                summary,
                shown,
                pinned,
                stepped,
                settled,
            } => {
                let row = drawing(
                    id,
                    summary,
                    (shown, pinned, stepped),
                    settled,
                    on_drawing.clone(),
                    cx,
                );
                rows.push(row);
                continue;
            }
            ChatEntry::Agent(text) => {
                rows.push(agent_words(index, text, on_link.clone(), cx));
                continue;
            }
            entry => {
                rows.push(render_entry(index, entry, cx));
                continue;
            }
        };
        let mut calls = vec![first];
        while let Some((_, ChatEntry::Did(_))) = entries.peek() {
            if let Some((_, ChatEntry::Did(text))) = entries.next() {
                calls.push(text);
            }
        }
        rows.push(if calls.len() == 1 {
            render_entry(index, ChatEntry::Did(calls.remove(0)), cx)
        } else {
            tool_calls(index, calls, window, cx)
        });
    }
    rows
}

/// The agent's words, in Markdown; a link clicked goes to `on_link` when
/// there is one.
fn agent_words(index: usize, text: String, on_link: Option<LinkHandler>, cx: &App) -> AnyElement {
    let theme = cx.theme();
    let code = StyleRefinement::default()
        .font_family(theme.mono_font_family.clone())
        .text_size(px(12.));
    let view = TextView::markdown(("jig-agent-text", index), text)
        .style(
            TextViewStyle::default()
                .paragraph_gap(rems(0.5))
                .code_block(code),
        )
        .selectable(true);
    match on_link {
        Some(on_link) => view
            .on_link_click(move |link, _, window, cx| on_link(link, window, cx))
            .into_any_element(),
        None => view.into_any_element(),
    }
}

/// A drawing on the Map: what it is, and buttons to show or hide it, pin
/// it and tour it.
fn drawing(
    id: usize,
    summary: String,
    (shown, pinned, stepped): (bool, bool, bool),
    settled: Option<String>,
    on_drawing: Option<DrawingHandler>,
    cx: &App,
) -> AnyElement {
    let theme = cx.theme();
    let accent = crate::surface::agent_accent(cx);
    let button = |name: &'static str, label: &'static str, action: DrawingAction, on: bool| {
        let handler = on_drawing.clone();
        div()
            .id((name, id))
            .debug_selector(move || format!("agent-drawing-{name}-{id}"))
            .flex_none()
            .px_1p5()
            .rounded(px(4.))
            .text_xs()
            .cursor_pointer()
            .text_color(if on { accent } else { theme.muted_foreground })
            .hover(|this| this.bg(theme.foreground.opacity(0.08)))
            .child(label)
            .when_some(handler, |this, handler| {
                this.on_click(move |_, window, cx| handler(&(id, action), window, cx))
            })
    };
    h_flex()
        .id(("jig-agent-drawing", id))
        .debug_selector(move || format!("agent-drawing-{id}"))
        .gap_1p5()
        .text_xs()
        .text_color(if shown {
            theme.foreground
        } else {
            theme.muted_foreground
        })
        .child(
            crate::surface::agent_icon()
                .size(px(11.))
                .flex_none()
                .text_color(if shown {
                    accent
                } else {
                    theme.muted_foreground
                }),
        )
        .child(div().flex_1().min_w_0().truncate().child(summary))
        .when_some(settled, |this, settled| {
            this.child(
                div()
                    .flex_none()
                    .text_color(theme.muted_foreground)
                    .child(settled),
            )
        })
        .when(stepped && shown, |this| {
            this.child(button("tour", "Tour", DrawingAction::Tour, false))
        })
        .child(button(
            "pin",
            if pinned { "Pinned" } else { "Pin" },
            DrawingAction::Pin,
            pinned,
        ))
        .child(button(
            "toggle",
            if shown { "Hide" } else { "Show" },
            DrawingAction::Toggle,
            shown,
        ))
        .into_any_element()
}

/// Tool calls in a row, folded to their count; a click lists them.
fn tool_calls(index: usize, calls: Vec<String>, window: &mut Window, cx: &mut App) -> AnyElement {
    let open = window.use_keyed_state(("jig-agent-tools-open", index), cx, |_, _| false);
    let is_open = *open.read(cx);
    let theme = cx.theme();
    let muted = theme.muted_foreground;
    let summary = h_flex()
        .id(("jig-agent-tools", index))
        .debug_selector(move || format!("agent-tools-{index}"))
        .gap_1()
        .text_xs()
        .text_color(muted)
        .cursor_pointer()
        .hover(|this| this.text_color(theme.foreground))
        .on_click(move |_, _, cx| {
            open.update(cx, |open, cx| {
                *open = !*open;
                cx.notify();
            })
        })
        .child(
            gpui_kit::component::Icon::default()
                .data(if is_open { CHEVRON_DOWN } else { CHEVRON })
                .size(px(11.))
                .text_color(muted),
        )
        .child(format!("Used {} tools", calls.len()));
    v_flex()
        .gap_1()
        .child(summary)
        .when(is_open, |this| {
            this.child(
                v_flex()
                    .debug_selector(move || format!("agent-tools-{index}-list"))
                    .ml(px(5.))
                    .pl(px(10.))
                    .gap_1()
                    .border_l_1()
                    .border_color(theme.border)
                    .text_xs()
                    .text_color(muted)
                    .children(calls),
            )
        })
        .into_any_element()
}

/// An edit's row, with how many lines it adds and removes; a click opens
/// its diff below it, in a box of its own that scrolls.
fn edit_with_diff(
    index: usize,
    path: String,
    accepted: bool,
    diff: Vec<DiffLine>,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let open = window.use_keyed_state(("jig-agent-edit-open", index), cx, |_, _| false);
    let is_open = *open.read(cx);
    let theme = cx.theme();
    let muted = theme.muted_foreground;
    let count = |kind: fn(&DiffLine) -> bool| diff.iter().filter(|line| kind(line)).count();
    let added = count(|line| matches!(line, DiffLine::Added(_)));
    let removed = count(|line| matches!(line, DiffLine::Removed(_)));
    let row = h_flex()
        .id(("jig-agent-edit", index))
        .debug_selector(move || format!("agent-edit-{index}"))
        .gap_1p5()
        .text_xs()
        .text_color(muted)
        .cursor_pointer()
        .on_click(move |_, _, cx| {
            open.update(cx, |open, cx| {
                *open = !*open;
                cx.notify();
            })
        })
        .child(
            gpui_kit::component::Icon::default()
                .data(if is_open { CHEVRON_DOWN } else { CHEVRON })
                .size(px(11.))
                .text_color(muted),
        )
        .child(if accepted {
            "Edited"
        } else {
            "Rejected an edit to"
        })
        .child(file_name(path, cx))
        .when(added > 0, |this| {
            this.child(div().text_color(theme.success).child(format!("+{added}")))
        })
        .when(removed > 0, |this| {
            this.child(div().text_color(theme.danger).child(format!("−{removed}")))
        });
    v_flex()
        .gap_1()
        .child(row)
        .when(is_open, |this| this.child(diff_view(index, diff, cx)))
        .into_any_element()
}

/// Tallest a subagent's work grows, opened, before it scrolls.
const SUBAGENT_HEIGHT: f32 = 260.;

/// A subagent's row: what it was given, how much it did, and a spinner
/// while it works; a click opens what it did and its report.
fn subagent(
    index: usize,
    description: String,
    calls: Vec<String>,
    report: Option<String>,
    on_link: Option<LinkHandler>,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let open = window.use_keyed_state(("jig-agent-subagent-open", index), cx, |_, _| false);
    let is_open = *open.read(cx);
    let theme = cx.theme();
    let muted = theme.muted_foreground;
    let accent = crate::surface::agent_accent(cx);
    let working = report.is_none();
    let count = match calls.len() {
        0 => String::new(),
        1 => "· 1 tool".into(),
        n => format!("· {n} tools"),
    };
    let row = h_flex()
        .id(("jig-agent-subagent", index))
        .debug_selector(move || format!("agent-subagent-{index}"))
        .gap_1p5()
        .text_xs()
        .text_color(muted)
        .cursor_pointer()
        .hover(|this| this.text_color(theme.foreground))
        .on_click(move |_, _, cx| {
            open.update(cx, |open, cx| {
                *open = !*open;
                cx.notify();
            })
        })
        .child(
            gpui_kit::component::Icon::default()
                .data(if is_open { CHEVRON_DOWN } else { CHEVRON })
                .size(px(11.))
                .text_color(muted),
        )
        .child(div().flex_none().text_color(accent).child("Subagent"))
        .child(div().min_w_0().truncate().child(description))
        .child(div().flex_none().child(count))
        .when(working, |this| {
            this.child(Spinner::new().color(accent).xsmall())
        });
    let body = is_open.then(|| {
        v_flex()
            .id(("jig-agent-subagent-work", index))
            .debug_selector(move || format!("agent-subagent-work-{index}"))
            .max_h(px(SUBAGENT_HEIGHT))
            .overflow_y_scroll()
            .ml(px(5.))
            .pl(px(10.))
            .gap_1()
            .border_l_1()
            .border_color(theme.border)
            .children(
                calls
                    .into_iter()
                    .map(|call| div().text_xs().text_color(muted).child(call)),
            )
            .when(working, |this| {
                this.child(div().text_xs().text_color(muted).child("Working…"))
            })
            .when_some(report, |this, report| {
                this.child(
                    v_flex()
                        .pt_1()
                        .gap_0p5()
                        .child(div().text_xs().text_color(muted).child("Reported"))
                        .child(div().text_sm().child({
                            let view =
                                TextView::markdown(("jig-agent-subagent-report", index), report)
                                    .style(TextViewStyle::default().paragraph_gap(rems(0.4)))
                                    .selectable(true);
                            match on_link.clone() {
                                Some(on_link) => view
                                    .on_link_click(move |link, _, window, cx| {
                                        on_link(link, window, cx)
                                    })
                                    .into_any_element(),
                                None => view.into_any_element(),
                            }
                        })),
                )
            })
    });
    v_flex()
        .gap_1()
        .child(row)
        .children(body)
        .into_any_element()
}

/// Tallest an edit's diff grows before it scrolls.
const DIFF_HEIGHT: f32 = 220.;

/// An edit's lines, removed in red and added in green, with the unchanged
/// lines around them. Long lines scroll sideways rather than wrap.
fn diff_view(index: usize, diff: Vec<DiffLine>, cx: &App) -> impl IntoElement {
    let theme = cx.theme();
    let line = |sign: &'static str, text: String, bg: Option<Hsla>, color: Hsla| {
        h_flex()
            .min_w_full()
            .px_1p5()
            .gap_1p5()
            .whitespace_nowrap()
            .when_some(bg, |this, bg| this.bg(bg))
            .child(div().flex_none().w(px(8.)).text_color(color).child(sign))
            .child(div().flex_none().text_color(color).child(text))
    };
    div()
        .id(("jig-agent-diff", index))
        .debug_selector(move || format!("agent-diff-{index}"))
        .max_h(px(DIFF_HEIGHT))
        .overflow_scroll()
        .py_1()
        .rounded(px(6.))
        .border_1()
        .border_color(theme.border)
        .bg(theme.foreground.opacity(0.03))
        .font_family(theme.mono_font_family.clone())
        .text_size(px(11.5))
        .child(
            v_flex()
                .min_w_full()
                .children(diff.into_iter().map(|diff_line| match diff_line {
                    DiffLine::Same(text) => line(" ", text, None, theme.muted_foreground),
                    DiffLine::Removed(text) => line(
                        "-",
                        text,
                        Some(theme.danger.opacity(0.12)),
                        theme.foreground,
                    ),
                    DiffLine::Added(text) => line(
                        "+",
                        text,
                        Some(theme.success.opacity(0.14)),
                        theme.foreground,
                    ),
                    DiffLine::Gap => line(" ", "⋯".into(), None, theme.muted_foreground),
                })),
        )
}

fn render_entry(index: usize, entry: ChatEntry, cx: &App) -> AnyElement {
    let theme = cx.theme();
    match entry {
        // The user's words sit on the right, tinted with the agent's colour.
        ChatEntry::User(text) => h_flex()
            .justify_end()
            .child(
                div()
                    .max_w(relative(0.85))
                    .px_2p5()
                    .py_1p5()
                    .rounded(px(9.))
                    .bg(crate::surface::agent_accent(cx).opacity(0.12))
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
        // Drawn by `subagent`, which keeps whether it's open.
        ChatEntry::Subagent { description, .. } => div()
            .text_xs()
            .text_color(theme.muted_foreground)
            .child(format!("Subagent: {description}"))
            .into_any_element(),
        ChatEntry::Did(text) => div()
            .text_xs()
            .text_color(theme.muted_foreground)
            .child(text)
            .into_any_element(),
        ChatEntry::Edit { path, accepted, .. } => h_flex()
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
        // Drawn by `drawing`, which has its buttons.
        ChatEntry::Drawing { summary, .. } => div()
            .text_xs()
            .text_color(theme.muted_foreground)
            .child(summary)
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
