//! Running a command: the palette, the AI request, and the reply bubble.

use std::ops::Range;
use std::sync::Arc;
use std::time::Duration;

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::input::{Enter, Escape, Indent, IndentInline, Undo};
use gpui_kit::*;
use jig_ai::{PromptRequest, Provider, Reply};
use jig_commands::{Bubble, CommandPalette, Invocation, LiveStep, PaletteEvent};
use jig_editor::EditorHandle;

use super::{OpenCommand, OpenPalette, Workspace};

/// How long an error stays up before it fades on its own.
const ERROR_TIMEOUT: Duration = Duration::from_secs(4);
/// Longest error text shown in the bubble.
const MAX_ERROR_CHARS: usize = 180;

/// Background of code a command is working on, while waiting for the model.
fn working_color(cx: &App) -> Hsla {
    cx.theme().primary.opacity(0.16)
}

/// Background of code a command just wrote, while it awaits review.
fn added_color(cx: &App) -> Hsla {
    cx.theme().success.opacity(0.18)
}

/// One command from the moment it's chosen until its bubble goes away.
pub(super) struct CommandRun {
    id: u64,
    pub(super) bubble: Bubble,
    pub(super) anchor: Point<Pixels>,
    /// The buffer when the command started. A reply for a buffer that has
    /// since changed is discarded.
    snapshot: String,
    pub(super) target: Range<usize>,
    /// Set while the change sits in the buffer awaiting accept or reject.
    pub(super) preview: Option<Preview>,
    _task: Option<Task<()>>,
}

pub(super) struct Preview {
    /// Where the new code is in the buffer.
    pub(super) range: Range<usize>,
}

pub(super) fn load_provider() -> Result<Arc<dyn Provider>, String> {
    jig_ai::Config::load(jig_ai::Config::user_path().as_deref())
        .and_then(|config| config.default_provider().build())
        .map_err(|error| format!("{error:#}"))
}

impl Workspace {
    /// Where floating UI opens: just below the cursor or selection.
    fn floating_anchor(&self, cx: &App) -> Point<Pixels> {
        self.editor()
            .anchor_point(cx)
            .map(|point| point + gpui_kit::point(px(-8.), px(4.)))
            // The cursor is scrolled out of view: open near the top instead.
            .unwrap_or(gpui_kit::point(px(48.), px(48.)))
    }

    pub(super) fn open_command(
        &mut self,
        _: &OpenCommand,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.palette.is_some() {
            return;
        }
        // A new command replaces whatever the last one left on screen; a
        // pending change counts as accepted.
        self.accept_preview(cx);
        self.run = None;
        self.editor().clear_highlights(cx);
        let has_selection = !self.editor().selection(cx).is_empty();
        let anchor = self.floating_anchor(cx);
        let presets = self.presets.clone();
        let view = cx.new(|cx| CommandPalette::new(presets, has_selection, window, cx));
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

        let provider = match &self.provider {
            Ok(provider) => provider.clone(),
            Err(error) => {
                let error = error.clone();
                self.run = Some(CommandRun {
                    id,
                    bubble: Bubble::Running {
                        label: String::new(),
                        started: std::time::Instant::now(),
                        with_rules: false,
                        step: None,
                    },
                    anchor,
                    snapshot: text,
                    target,
                    preview: None,
                    _task: None,
                });
                self.fail(id, error, window, cx);
                return;
            }
        };

        let request = PromptRequest {
            instruction: invocation.instruction.clone(),
            language: self.editor().language(cx),
            file_name: self.document().path.as_deref().map(|path| {
                // Relative to the project, so an exploring model knows
                // where the file sits.
                let root = crate::project::root_for(path);
                path.strip_prefix(&root)
                    .unwrap_or(path)
                    .to_string_lossy()
                    .into_owned()
            }),
            text: text.clone(),
            target: target.clone(),
            comment: invocation.comment.clone(),
            project_rules: self
                .document()
                .path
                .as_deref()
                .and_then(crate::project::rules_for),
        };
        let with_rules = request.project_rules.is_some();
        // Exploring needs a saved file, to know which project to look in.
        let tools = self
            .document()
            .path
            .as_deref()
            .filter(|_| invocation.explore)
            .and_then(|path| jig_ai::ProjectTools::new(&crate::project::root_for(path)).ok());
        let step = tools.as_ref().map(|_| LiveStep::default());
        let live = step.clone();
        let task = cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    match (&tools, &live) {
                        (Some(tools), Some(live)) => {
                            jig_ai::run_exploring(provider.as_ref(), &request, tools, &|text| {
                                live.set(text)
                            })
                        }
                        _ => jig_ai::run(provider.as_ref(), &request),
                    }
                })
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
                started: std::time::Instant::now(),
                with_rules,
                step,
            },
            anchor,
            snapshot: text,
            target,
            preview: None,
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
        // Mark the preview first so the edit's change event doesn't dismiss it.
        run.preview = Some(Preview {
            range: run.target.clone(),
        });
        run.bubble = Bubble::Preview {
            message: reply.message,
            removed: original,
        };
        let target = run.target.clone();
        let range = self.editor().apply_edit(target, &reply.replace, window, cx);
        self.editor().set_readonly(true, cx);
        self.editor()
            .highlight(vec![(range.clone(), added_color(cx))], cx);
        if let Some(preview) = self.run.as_mut().and_then(|run| run.preview.as_mut()) {
            preview.range = range;
        }
        cx.notify();
    }

    /// The command input or the new-command form has the keyboard.
    fn modal_open(&self) -> bool {
        self.palette.is_some() || self.new_command.is_some()
    }

    pub(super) fn previewing(&self) -> bool {
        self.run.as_ref().is_some_and(|run| run.preview.is_some())
    }

    /// Keep the change. It is already in the buffer as one undo step.
    pub(super) fn accept_preview(&mut self, cx: &mut Context<Self>) -> bool {
        if !self.previewing() {
            return false;
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

    pub(super) fn on_accept_tab(
        &mut self,
        _: &IndentInline,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.accept_or_propagate(cx);
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
    fn fail(&mut self, id: u64, error: String, window: &mut Window, cx: &mut Context<Self>) {
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

    /// Esc rejects a pending change, cancels a running command or dismisses
    /// its bubble. With the palette open, the palette handles Esc itself.
    pub(super) fn on_escape(&mut self, _: &Escape, window: &mut Window, cx: &mut Context<Self>) {
        if self.modal_open() {
            cx.propagate();
        } else if self.reject_preview(window, cx) || self.run.take().is_some() {
            self.editor().clear_highlights(cx);
            cx.stop_propagation();
            cx.notify();
        } else {
            cx.propagate();
        }
    }

    /// Any edit makes a shown reply stale.
    pub(super) fn on_buffer_changed(&mut self, cx: &mut Context<Self>) {
        if self
            .run
            .as_ref()
            .is_some_and(|run| !run.bubble.is_running() && run.preview.is_none())
        {
            self.run = None;
            cx.notify();
        }
    }
}

fn shorten(error: &str) -> String {
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
