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
    /// The code the conversation started from.
    pub(super) target: Range<usize>,
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

    /// Where a floating panel `width` wide opens: just right of the code in
    /// `range`, level with its first line, so it covers none of it, when
    /// there's room for it before the window's edge (or a docked
    /// conversation); below the cursor otherwise.
    pub(super) fn beside_anchor(
        &self,
        range: Range<usize>,
        width: Pixels,
        window: &Window,
        cx: &App,
    ) -> Point<Pixels> {
        const GAP: f32 = 32.;
        const MARGIN: f32 = 8.;
        let edge = window.viewport_size().width - self.docked_chat_width();
        self.editor()
            .beside_point(range, cx)
            // Level with the first line, give or take the panel's padding.
            .map(|at| at + gpui_kit::point(px(GAP), px(-4.)))
            .filter(|at| at.x + width + px(MARGIN) <= edge)
            .unwrap_or_else(|| self.floating_anchor(cx))
    }

    /// A button in the title bar that opens the agent input, for those who
    /// reach for the mouse or don't know ⌘K yet. Only with a file open.
    pub(super) fn render_command_button(&self, cx: &Context<Self>) -> Option<AnyElement> {
        if self.home {
            return None;
        }
        let theme = cx.theme();
        let accent = jig_commands::surface::agent_accent(cx);
        // Pressed while the palette is open: it toggles. It glows as the
        // palette opens, so the two read as one.
        let opened = self.palette.as_ref().map(|palette| palette.serial);
        let shortcut = if cfg!(target_os = "macos") {
            "⌘K"
        } else {
            "Ctrl K"
        };
        Some(
            h_flex()
                .id("open-command")
                .debug_selector(|| "open-command".into())
                .relative()
                .flex_none()
                .ml_2()
                .h(px(26.))
                .px_2()
                .gap_1p5()
                .rounded(px(6.))
                .text_size(px(12.5))
                .text_color(theme.foreground.opacity(0.85))
                // Behind the label: bright as the palette opens, settling to
                // the pressed tint.
                .children(opened.map(|serial| {
                    div()
                        .absolute()
                        .top_0()
                        .left_0()
                        .size_full()
                        .rounded(px(6.))
                        .with_animation(
                            ("agent-button-glow", serial),
                            Animation::new(Duration::from_millis(420))
                                .with_easing(ease_out_quint()),
                            move |glow, delta| glow.bg(accent.opacity(0.38 - 0.22 * delta)),
                        )
                }))
                .when(opened.is_some(), |s| s.text_color(accent))
                .when(opened.is_none(), |s| {
                    s.hover(|s| s.bg(theme.foreground.opacity(0.08)))
                })
                .child(
                    jig_commands::surface::agent_icon()
                        .size(px(13.))
                        .flex_none()
                        .text_color(accent),
                )
                .child(div().flex_none().whitespace_nowrap().child("Agent"))
                .child(
                    div()
                        .flex_none()
                        .text_size(px(11.))
                        .text_color(theme.muted_foreground)
                        .child(shortcut),
                )
                // Otherwise the title bar takes the press as a window drag.
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(
                    cx.listener(|this, _, window, cx| this.open_command(&OpenCommand, window, cx)),
                )
                .into_any_element(),
        )
    }

    pub(super) fn open_command(
        &mut self,
        _: &OpenCommand,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.home {
            return;
        }
        // ⌘K and the Agent button toggle it.
        if self.palette.is_some() {
            return self.close_palette(window, cx);
        }
        self.quick_open = None;
        self.find_in_files = None;
        // Its matches would muddle the change's highlights.
        self.close_find_panel(cx);
        self.note = None;
        let selection = self.editor().selection(cx);
        let has_selection = !selection.is_empty();
        // Beside the selection, or the cursor's line.
        let anchor = self.beside_anchor(selection, px(jig_commands::palette::WIDTH), window, cx);
        let models = self.model_menu(cx);
        let view = cx.new(|cx| CommandPalette::new(has_selection, window, cx).with_models(models));
        let events = cx.subscribe_in(
            &view,
            window,
            |this, palette, event: &PaletteEvent, window, cx| {
                if let PaletteEvent::PickModel(choice) = event {
                    return this.pick_model(choice.clone(), palette.clone(), window, cx);
                }
                this.close_palette(window, cx);
                match event {
                    PaletteEvent::Run(text) => {
                        this.start_conversation(Ask::new(text.clone()), window, cx)
                    }
                    PaletteEvent::Resume(id) => this.resume_agent(id.clone(), window, cx),
                    PaletteEvent::RunAgentCommand { name, arguments } => {
                        this.run_agent_command(name.clone(), arguments.clone(), window, cx)
                    }
                    PaletteEvent::PickModel(_) | PaletteEvent::Dismissed => {}
                }
            },
        );
        self.load_agent_catalog(Some(view.clone()), window, cx);
        self.palette_openings += 1;
        self.palette = Some(OpenPalette {
            view,
            anchor,
            serial: self.palette_openings,
            _events: events,
        });
        cx.notify();
    }

    fn close_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.palette.take().is_some() {
            self.editor().focus(window, cx);
            cx.notify();
        }
    }

    /// Start a conversation about the selection, or the cursor's line, in
    /// place of the one on screen.
    pub(super) fn start_conversation(
        &mut self,
        ask: Ask,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Two changes under review in one buffer would tangle their undo.
        if self.agent_edit_here() {
            return self.show_note(
                "Accept or reject the agent's edit first.".into(),
                window,
                cx,
            );
        }
        let text = self.editor().text(cx);
        let selection = self.editor().selection(cx);
        let target = if selection.is_empty() {
            line_at(&text, selection.start)
        } else {
            selection
        };
        let request = jig_ai::agent::Prompt {
            instruction: ask.instruction,
            language: self.editor().language(cx),
            file_name: self.document().path.as_deref().map(|path| {
                // Relative to the project, so the agent knows where the
                // file sits.
                let root = crate::project::root_for(path);
                path.strip_prefix(&root)
                    .unwrap_or(path)
                    .to_string_lossy()
                    .into_owned()
            }),
            diagnostics: if ask.diagnostics {
                self.problems_on_lines(&text, target.clone(), cx)
            } else {
                Vec::new()
            },
            task: ask.task,
            text,
            target: target.clone(),
        };
        self.close_chat(window, cx);
        self.next_run_id += 1;
        let id = self.next_run_id;
        self.editor()
            .highlight(vec![(target.clone(), working_color(cx))], cx);
        self.chat = Some(CommandRun {
            id,
            bubble: Bubble::Message(String::new()),
            anchor: self.floating_anchor(cx),
            target,
            preview: None,
            agent: None,
            _task: None,
        });
        self.run_agent(id, ask.name, request, window, cx);
    }

    /// The project files the agent reads besides the code, e.g.
    /// "AGENTS.md", by name.
    pub(super) fn context_names(&self) -> Vec<String> {
        self.document()
            .path
            .as_deref()
            .map(crate::project::context_names)
            .unwrap_or_default()
    }

    /// The agent input or a form has the keyboard.
    pub(super) fn modal_open(&self) -> bool {
        self.palette.is_some()
            || self.planner.form.is_some()
            || self.quick_open.is_some()
            || self.find_in_files.is_some()
    }

    /// The agent's edit is under review in the tab on screen. One in
    /// another tab waits there.
    pub(super) fn previewing(&self) -> bool {
        self.agent_edit_here()
    }

    /// The agent's edit is under review in the tab on screen.
    pub(super) fn agent_edit_here(&self) -> bool {
        self.chat
            .as_ref()
            .is_some_and(|chat| chat.preview.is_some())
            && self.agent_edit_elsewhere().is_none()
    }

    /// Keep the edit on screen. It is already in the buffer as one undo
    /// step.
    pub(super) fn accept_preview(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        self.agent_edit_here() && self.settle_agent_edit(true, Some(window), cx)
    }

    /// Drop the edit on screen by undoing it, which leaves no trace in the
    /// history beyond a redo step.
    fn reject_preview(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        self.agent_edit_here() && self.settle_agent_edit(false, Some(window), cx)
    }

    /// Enter accepts or allows; ⌘↩ allows the agent's request always.
    pub(super) fn on_accept_enter(
        &mut self,
        action: &Enter,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if action.secondary
            && !self.keys_elsewhere(window, cx)
            && self.allow_agent_request_always(window, cx)
        {
            cx.stop_propagation();
            return;
        }
        self.accept_or_propagate(window, cx);
    }

    /// Whether the review keys belong to something else: a modal, or the
    /// find panel, where Enter goes to the next match and Esc closes it.
    pub(super) fn keys_elsewhere(&self, window: &Window, cx: &App) -> bool {
        self.modal_open() || self.find_panel_focused(window, cx) || self.planner_focused(window, cx)
    }

    /// Tab accepts a change under review, or moves on in a snippet.
    pub(super) fn on_accept_tab(
        &mut self,
        _: &IndentInline,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.keys_elsewhere(window, cx)
            && (self.settle_agent_request(true, window, cx)
                || self.accept_preview(window, cx)
                || self.snippet_step(true, cx))
        {
            cx.stop_propagation();
        } else {
            cx.propagate();
        }
    }

    /// Shift-Tab goes back in a snippet.
    pub(super) fn on_shift_tab(
        &mut self,
        _: &OutdentInline,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.keys_elsewhere(window, cx) && self.snippet_step(false, cx) {
            cx.stop_propagation();
        } else {
            cx.propagate();
        }
    }

    pub(super) fn on_accept_indent(
        &mut self,
        _: &Indent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.accept_or_propagate(window, cx);
    }

    /// Enter or Tab: accept the change under review, or allow what the
    /// agent asks to do.
    fn accept_or_propagate(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.keys_elsewhere(window, cx)
            && (self.settle_agent_request(true, window, cx) || self.accept_preview(window, cx))
        {
            cx.stop_propagation();
        } else {
            cx.propagate();
        }
    }

    /// Cmd+Z during review rejects the change.
    pub(super) fn on_undo(&mut self, _: &Undo, window: &mut Window, cx: &mut Context<Self>) {
        if !self.keys_elsewhere(window, cx) && self.reject_preview(window, cx) {
            cx.stop_propagation();
        } else {
            cx.propagate();
        }
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

    /// Esc rejects a pending edit or what the agent asks to do, dismisses a
    /// note, goes back to an agent edit waiting in another tab, stops the
    /// agent's turn, or closes the conversation.
    /// With the palette open, the palette handles Esc itself.
    pub(super) fn on_escape(&mut self, _: &Escape, window: &mut Window, cx: &mut Context<Self>) {
        if self.modal_open() && self.editor().state().focus_handle(cx).is_focused(window) {
            // Focus went back to the code (e.g. a click outside the form or
            // the input), so the floating window can't hear Escape.
            self.palette = None;
            self.planner.form = None;
            self.quick_open = None;
            self.find_in_files = None;
            cx.stop_propagation();
            cx.notify();
        } else if self.keys_elsewhere(window, cx) || self.quick_fix_open(cx) {
            // The Quick Fix list closes itself.
            cx.propagate();
        } else if self.reject_preview(window, cx)
            || self.settle_agent_request(false, window, cx)
            || self.close_hunk_popup(cx)
            || self.close_find_panel(cx)
            || self.close_code_hover(cx)
            || self.close_problem(cx)
        {
            cx.stop_propagation();
        } else if self.note.take().is_some() {
            cx.stop_propagation();
            cx.notify();
        } else if self.code_takes_escape(cx) {
            // The first Escape closes the completion list, the next leaves
            // the snippet; neither stops the agent.
            let listing = self.editor().state().read(cx).completion_menu_state().open;
            if !listing {
                self.end_snippet();
            }
            cx.propagate();
        } else if self.escape_agent(window, cx) {
            cx.stop_propagation();
        } else {
            cx.propagate();
        }
    }

    /// The code has a completion list open or a snippet being filled in,
    /// which Esc closes before anything else.
    fn code_takes_escape(&self, cx: &App) -> bool {
        self.editor().state().read(cx).completion_menu_state().open
            || self
                .tabs
                .get(self.active)
                .is_some_and(|tab| tab.snippet.is_some())
    }

    /// Any edit makes a note stale.
    pub(super) fn on_buffer_changed(&mut self, cx: &mut Context<Self>) {
        if self.note.take().is_some() {
            cx.notify();
        }
    }
}

/// The line containing `offset`, without its trailing newline.
fn line_at(text: &str, offset: usize) -> Range<usize> {
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
