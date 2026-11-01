//! The agent conversation: a floating window at the code with what was asked,
//! what the agent said and did, and a box to reply in. Each reply continues
//! the same agent session, so the agent remembers the turns before. It can
//! be pinned to the window's right edge, and its header opens the earlier
//! conversations.

mod composer;
mod details;
mod status;
mod transcript;
mod view;

use std::cell::Cell;
use std::rc::Rc;

use gpui_kit::component::input::{Enter, TextareaState};
use gpui_kit::*;

pub use crate::domain::transcript::{
    ChatEntry, ChatModels, ChatStatus, DiffLine, DrawingAction, LiveStep,
};

pub use composer::message_input;

/// The conversation's width.
pub const WIDTH: f32 = 460.;

/// Tallest the transcript grows before it scrolls.
const TRANSCRIPT_HEIGHT: f32 = 280.;

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
