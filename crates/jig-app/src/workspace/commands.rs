//! Running a command: the palette, the AI request, and the reply bubble.

use std::ops::Range;
use std::time::Duration;

use gpui_kit::component::input::{Enter, Escape, Indent, IndentInline, OutdentInline, Undo};
use gpui_kit::component::{ActiveTheme as _, Icon, h_flex};
use gpui_kit::*;
use jig_ai::{PromptRequest, Reply};
use jig_commands::{Bubble, CommandPalette, Invocation, PaletteEvent};
use jig_editor::EditorHandle;

use super::agent::AgentRun;
use super::{OpenCommand, OpenPalette, Workspace};

/// The title bar's button for the command input (Lucide "sparkles").
pub(super) const COMMAND_ICON: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M9.937 15.5A2 2 0 0 0 8.5 14.063l-6.135-1.582a.5.5 0 0 1 0-.962L8.5 9.936A2 2 0 0 0 9.937 8.5l1.582-6.135a.5.5 0 0 1 .963 0L14.063 8.5A2 2 0 0 0 15.5 9.937l6.135 1.581a.5.5 0 0 1 0 .964L15.5 14.063a2 2 0 0 0-1.437 1.437l-1.582 6.135a.5.5 0 0 1-.963 0z"/><path d="M20 3v4"/><path d="M22 5h-4"/></svg>"#;

/// How long an error stays up before it fades on its own.
const ERROR_TIMEOUT: Duration = Duration::from_secs(4);
/// Longest error text shown in the bubble.
const MAX_ERROR_CHARS: usize = 180;

/// Background of code a command is working on, while waiting for the model.
fn working_color(cx: &App) -> Hsla {
    cx.theme().primary.opacity(0.16)
}

/// Background of code a command just wrote, while it awaits review.
pub(super) fn added_color(cx: &App) -> Hsla {
    cx.theme().success.opacity(0.18)
}

/// One command from the moment it's chosen until its bubble goes away.
pub(super) struct CommandRun {
    pub(super) id: u64,
    pub(super) bubble: Bubble,
    pub(super) anchor: Point<Pixels>,
    /// The buffer when the command started. A reply for a buffer that has
    /// since changed is discarded.
    snapshot: String,
    pub(super) target: Range<usize>,
    /// Set while the change sits in the buffer awaiting accept or reject.
    pub(super) preview: Option<Preview>,
    /// Set for a command handed to the agent.
    pub(super) agent: Option<AgentRun>,
    pub(super) _task: Option<Task<()>>,
}

pub(super) struct Preview {
    /// Where the new code is in the buffer.
    pub(super) range: Range<usize>,
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

    /// A button in the title bar that opens the command input, for those
    /// who reach for the mouse or don't know ⌘K yet. Only with a file open.
    pub(super) fn render_command_button(&self, cx: &Context<Self>) -> Option<AnyElement> {
        if self.home {
            return None;
        }
        let theme = cx.theme();
        let accent = jig_commands::surface::lane_accent(false, cx);
        let shortcut = if cfg!(target_os = "macos") {
            "⌘K"
        } else {
            "Ctrl K"
        };
        Some(
            h_flex()
                .id("open-command")
                .debug_selector(|| "open-command".into())
                .flex_none()
                .ml_2()
                .h(px(26.))
                .px_2()
                .gap_1p5()
                .rounded(px(6.))
                .text_size(px(12.5))
                .text_color(theme.foreground.opacity(0.85))
                .hover(|s| s.bg(theme.foreground.opacity(0.08)))
                .child(
                    Icon::default()
                        .data(COMMAND_ICON)
                        .size(px(13.))
                        .flex_none()
                        .text_color(accent),
                )
                .child(div().flex_none().whitespace_nowrap().child("Command"))
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
        if self.palette.is_some() || self.home {
            return;
        }
        self.quick_open = None;
        self.find_in_files = None;
        // A new command replaces whatever the last one left on screen; a
        // pending change counts as accepted.
        self.accept_preview(cx);
        self.run = None;
        self.editor().clear_highlights(cx);
        let has_selection = !self.editor().selection(cx).is_empty();
        let anchor = self.floating_anchor(cx);
        let presets = self.presets.clone();
        let context = self.context_names();
        let view = cx.new(|cx| {
            CommandPalette::new(presets, has_selection, window, cx).with_context(context)
        });
        let events = cx.subscribe_in(
            &view,
            window,
            |this, _, event: &PaletteEvent, window, cx| {
                this.close_palette(window, cx);
                match event {
                    PaletteEvent::Run(invocation) => {
                        this.run_command(invocation.clone(), window, cx)
                    }
                    PaletteEvent::SaveAsCommand(text) => {
                        this.open_add_command(Some(text.clone()), window, cx)
                    }
                    PaletteEvent::Dismissed => {}
                }
            },
        );
        self.palette = Some(OpenPalette {
            view,
            anchor,
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

    pub(super) fn run_command(
        &mut self,
        invocation: Invocation,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let text = self.editor().text(cx);
        let target =
            invocation
                .scope
                .target(&text, self.editor().selection(cx), self.editor().cursor(cx));
        let anchor = self.floating_anchor(cx);
        self.next_run_id += 1;
        let id = self.next_run_id;

        let request = PromptRequest {
            instruction: invocation.instruction.clone(),
            language: self.editor().language(cx),
            file_name: self.document().path.as_deref().map(|path| {
                // Relative to the project, so the model knows where the
                // file sits.
                let root = crate::project::root_for(path);
                path.strip_prefix(&root)
                    .unwrap_or(path)
                    .to_string_lossy()
                    .into_owned()
            }),
            text: text.clone(),
            target: target.clone(),
            comment: invocation.comment.clone(),
            // The agent reads AGENTS.md itself.
            project_rules: self
                .document()
                .path
                .as_deref()
                .filter(|_| !invocation.agent)
                .and_then(crate::project::agents_for),
        };
        if invocation.agent {
            self.editor()
                .highlight(vec![(target.clone(), working_color(cx))], cx);
            self.run = Some(CommandRun {
                id,
                bubble: Bubble::Message(String::new()),
                anchor,
                snapshot: text,
                target,
                preview: None,
                agent: None,
                _task: None,
            });
            self.run_agent(id, invocation.name, request, window, cx);
            return;
        }
        let provider = match &self.provider {
            Ok(provider) => provider.clone(),
            Err(error) => {
                let error = error.clone();
                self.run = Some(CommandRun {
                    id,
                    bubble: Bubble::Running {
                        label: String::new(),
                        agent: false,
                        started: std::time::Instant::now(),
                        context: Vec::new(),
                        step: None,
                    },
                    anchor,
                    snapshot: text,
                    target,
                    preview: None,
                    agent: None,
                    _task: None,
                });
                self.fail(id, error, window, cx);
                return;
            }
        };

        let context = self.context_names();
        let task = cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { jig_ai::run(provider.as_ref(), &request) })
                .await;
            this.update_in(cx, |this, window, cx| this.finish(id, result, window, cx))
                .ok();
        });

        let label = invocation.name.unwrap_or_else(|| "Working".into());
        // Tint the code being worked on until the reply arrives.
        self.editor()
            .highlight(vec![(target.clone(), working_color(cx))], cx);
        self.run = Some(CommandRun {
            id,
            bubble: Bubble::Running {
                label,
                agent: false,
                started: std::time::Instant::now(),
                context,
                step: None,
            },
            anchor,
            snapshot: text,
            target,
            preview: None,
            agent: None,
            _task: Some(task),
        });
        cx.notify();
    }

    fn finish(
        &mut self,
        id: u64,
        result: anyhow::Result<Reply>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(run) = self.run.as_ref().filter(|run| run.id == id) else {
            return; // Cancelled or replaced while waiting.
        };
        if self.editor().text(cx) != run.snapshot {
            self.fail(
                id,
                "The file changed while waiting, so nothing was applied.".into(),
                window,
                cx,
            );
            return;
        }
        match result {
            Ok(reply) => self.show_reply(reply, window, cx),
            Err(error) => self.fail(id, format!("{error:#}"), window, cx),
        }
    }

    /// Put the reply's code into the buffer for review: highlighted,
    /// read-only, and one undo step away from the original.
    fn show_reply(&mut self, reply: Reply, window: &mut Window, cx: &mut Context<Self>) {
        let Some(run) = self.run.as_mut() else { return };
        let original = run.snapshot[run.target.clone()].to_string();
        if reply.replace == original {
            self.editor().clear_highlights(cx);
            let Some(run) = self.run.as_mut() else { return };
            run.bubble = Bubble::Message(if reply.message.is_empty() {
                "No changes.".into()
            } else {
                reply.message
            });
            cx.notify();
            return;
        }
        // Only what changed: the edit leaves identical text alone, the new
        // lines are highlighted and only lines really removed are listed.
        let edit = crate::diff::preview_edit(&original, &reply.replace, run.target.start);
        // Mark the preview first so the edit's change event doesn't dismiss it.
        run.preview = Some(Preview {
            range: edit.range.clone(),
        });
        run.bubble = Bubble::Preview {
            message: reply.message,
            removed: edit.removed,
            agent: false,
        };
        let range = self
            .editor()
            .apply_edit(edit.range, edit.replacement, window, cx);
        self.editor().set_readonly(true, cx);
        let color = added_color(cx);
        self.editor().highlight(
            edit.highlights
                .into_iter()
                .map(|range| (range, color))
                .collect(),
            cx,
        );
        if let Some(preview) = self.run.as_mut().and_then(|run| run.preview.as_mut()) {
            preview.range = range;
        }
        cx.notify();
    }

    /// The project files commands are sent with, e.g. "AGENTS.md", by name.
    pub(super) fn context_names(&self) -> Vec<String> {
        self.document()
            .path
            .as_deref()
            .map(crate::project::context_names)
            .unwrap_or_default()
    }

    /// The command input or the new-command form has the keyboard.
    pub(super) fn modal_open(&self) -> bool {
        self.palette.is_some()
            || self.new_command.is_some()
            || self.quick_open.is_some()
            || self.find_in_files.is_some()
    }

    pub(super) fn previewing(&self) -> bool {
        self.run.as_ref().is_some_and(|run| run.preview.is_some())
    }

    /// Keep the change. It is already in the buffer as one undo step.
    pub(super) fn accept_preview(&mut self, cx: &mut Context<Self>) -> bool {
        if !self.previewing() {
            return false;
        }
        if self.settle_agent_edit(true, None, cx) {
            return true;
        }
        self.run = None;
        self.editor().clear_highlights(cx);
        self.editor().set_readonly(false, cx);
        cx.notify();
        true
    }

    /// Drop the change by undoing it, which leaves no trace in the history
    /// beyond a redo step.
    fn reject_preview(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if !self.previewing() {
            return false;
        }
        if self.settle_agent_edit(false, Some(window), cx) {
            return true;
        }
        self.run = None;
        self.editor().clear_highlights(cx);
        self.editor().set_readonly(false, cx);
        self.editor().undo(window, cx);
        cx.notify();
        true
    }

    pub(super) fn on_accept_enter(&mut self, _: &Enter, _: &mut Window, cx: &mut Context<Self>) {
        self.accept_or_propagate(cx);
    }

    /// Tab accepts a change under review, or moves on in a snippet.
    pub(super) fn on_accept_tab(
        &mut self,
        _: &IndentInline,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.modal_open() && (self.accept_preview(cx) || self.snippet_step(true, cx)) {
            cx.stop_propagation();
        } else {
            cx.propagate();
        }
    }

    /// Shift-Tab goes back in a snippet.
    pub(super) fn on_shift_tab(
        &mut self,
        _: &OutdentInline,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.modal_open() && self.snippet_step(false, cx) {
            cx.stop_propagation();
        } else {
            cx.propagate();
        }
    }

    pub(super) fn on_accept_indent(&mut self, _: &Indent, _: &mut Window, cx: &mut Context<Self>) {
        self.accept_or_propagate(cx);
    }

    fn accept_or_propagate(&mut self, cx: &mut Context<Self>) {
        if !self.modal_open() && self.accept_preview(cx) {
            cx.stop_propagation();
        } else {
            cx.propagate();
        }
    }

    /// Cmd+Z during review rejects the change.
    pub(super) fn on_undo(&mut self, _: &Undo, window: &mut Window, cx: &mut Context<Self>) {
        if !self.modal_open() && self.reject_preview(window, cx) {
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
        self.run = Some(CommandRun {
            id,
            bubble: Bubble::Message(message),
            anchor: self.floating_anchor(cx),
            snapshot: String::new(),
            target: 0..0,
            preview: None,
            agent: None,
            _task: Some(cx.spawn_in(window, async move |this, cx| {
                cx.background_executor().timer(ERROR_TIMEOUT).await;
                this.update(cx, |this, cx| {
                    if this.run.as_ref().is_some_and(|run| run.id == id) {
                        this.run = None;
                        cx.notify();
                    }
                })
                .ok();
            })),
        });
        cx.notify();
    }

    /// Show `error` in the run's bubble and dismiss it after a few seconds.
    pub(super) fn fail(
        &mut self,
        id: u64,
        error: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.run.as_ref().is_some_and(|run| run.id == id) {
            return;
        }
        self.editor().clear_highlights(cx);
        let Some(run) = self.run.as_mut() else { return };
        run.bubble = Bubble::Error(shorten(&error));
        run._task = Some(cx.spawn_in(window, async move |this, cx| {
            cx.background_executor().timer(ERROR_TIMEOUT).await;
            this.update(cx, |this, cx| {
                if this.run.as_ref().is_some_and(|run| run.id == id) {
                    this.run = None;
                    cx.notify();
                }
            })
            .ok();
        }));
        cx.notify();
    }

    /// Esc rejects a pending change, stops the agent's turn, cancels a
    /// running command or dismisses its bubble or the agent conversation.
    /// With the palette open, the palette handles Esc itself.
    pub(super) fn on_escape(&mut self, _: &Escape, window: &mut Window, cx: &mut Context<Self>) {
        if self.modal_open() && self.editor().state().focus_handle(cx).is_focused(window) {
            // Focus went back to the code (e.g. a click outside the form or
            // command input), so the floating window can't hear Escape.
            self.palette = None;
            self.new_command = None;
            self.quick_open = None;
            self.find_in_files = None;
            cx.stop_propagation();
            cx.notify();
        } else if self.modal_open() {
            cx.propagate();
        } else if self.reject_preview(window, cx)
            || self.stop_agent_turn(window, cx)
            || self.close_hunk_popup(cx)
        {
            cx.stop_propagation();
        } else if self.run.take().is_some() {
            self.editor().clear_highlights(cx);
            // The agent's reply box may have had the keyboard.
            self.editor().focus(window, cx);
            cx.stop_propagation();
            cx.notify();
        } else {
            // The first Escape closes the completion list, the next leaves
            // the snippet.
            let listing = self.editor().state().read(cx).completion_menu_state().open;
            if !listing {
                self.end_snippet();
            }
            cx.propagate();
        }
    }

    /// Any edit makes a shown reply stale.
    pub(super) fn on_buffer_changed(&mut self, cx: &mut Context<Self>) {
        if self
            .run
            .as_ref()
            // The agent's conversation stays until it's closed.
            .is_some_and(|run| {
                !run.bubble.is_running() && run.preview.is_none() && run.agent.is_none()
            })
        {
            self.run = None;
            cx.notify();
        }
    }
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
