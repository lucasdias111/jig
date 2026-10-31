//! The agent's edits: writing them, showing each for review, and settling it.

use super::*;

impl Workspace {
    /// Accept the agent's edit without showing it, its edits having been
    /// accepted for the conversation: written to disk (by Jig or by the
    /// agent once told), its open tab caught up, and listed. False, with
    /// nothing done, when its open tab has unsaved changes.
    pub(super) fn write_agent_edit(
        &mut self,
        edit: &EditRequest,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<bool, String> {
        let tab = self.tab_for(&edit.path);
        if tab.is_some_and(|ix| self.tabs[ix].dirty) {
            return Ok(false);
        }
        let created = !edit.path.exists();
        let disk = std::fs::read_to_string(&edit.path).unwrap_or_default();
        let new = edit.change.apply(&disk).map_err(|error| {
            format!("Jig couldn't apply that edit: {error:#}. Read the file again.")
        })?;
        if edit.change.written_by_jig() {
            std::fs::write(&edit.path, &new)
                .map_err(|error| format!("Jig couldn't write the file: {error}"))?;
            if let Some(ix) = tab {
                self.catch_up_with_disk(ix, window, cx);
            }
        }
        if created {
            self.refresh_tree(cx);
        }
        self.answer_edit(&edit.id, true, None, cx);
        let path = self.relative(&edit.path);
        let diff = jig_kit::diff::edit_lines(&disk, &new);
        if let Some(agent) = self.chat.as_mut().and_then(|run| run.agent.as_mut()) {
            agent.entries.push(ChatEntry::Edit {
                path,
                accepted: true,
                diff,
            });
            // Written by the agent: caught up with when the turn ends.
            agent.touched.push(edit.path.clone());
            agent.scroll.scroll_to_bottom();
        }
        cx.notify();
        Ok(true)
    }

    pub(super) fn show_edit(
        &mut self,
        edit: &EditRequest,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        let created = !edit.path.exists();
        let disk = std::fs::read_to_string(&edit.path).unwrap_or_default();
        let new = edit.change.apply(&disk).map_err(|error| {
            format!("Jig couldn't apply that edit: {error:#}. Read the file again.")
        })?;
        if created {
            std::fs::write(&edit.path, "").map_err(|error| error.to_string())?;
            self.refresh_tree(cx);
        }

        let from_agent = self.mode == super::super::mode::Mode::Agent;
        if from_agent {
            self.remember_location(cx);
        }
        // Show the file, keeping the run alive across the tab switch.
        let on_screen = self
            .document()
            .path
            .as_deref()
            .is_some_and(|open| canonical(open) == canonical(&edit.path));
        if !on_screen {
            let run = self.chat.take();
            self.open_file(&edit.path, window, cx);
            self.chat = run;
        }
        if self.tab().dirty {
            return Err("The user has unsaved changes in this file; leave it alone.".into());
        }
        self.catch_up_with_disk(self.active, window, cx);

        let old = self.editor().text(cx);
        let preview = jig_kit::diff::preview_edit(&old, &new, 0);
        let relative = self.relative(&edit.path);
        let Some(run) = self.chat.as_mut() else {
            return Ok(());
        };
        run.preview = Some(Preview {
            range: preview.range.clone(),
        });
        run.bubble = Bubble::Preview {
            message: relative,
            removed: preview.removed,
        };
        if let Some(agent) = run.agent.as_mut() {
            agent.pending = Some(Pending::Edit(PendingEdit {
                id: edit.id.clone(),
                path: edit.path.clone(),
                created,
                jig_writes: edit.change.written_by_jig(),
                before: old.clone(),
            }));
        }
        self.editor().clear_highlights(cx);
        let range = self
            .editor()
            .apply_edit(preview.range, preview.replacement, window, cx);
        self.editor().set_readonly(true, cx);
        let color = super::super::commands::added_color(cx);
        self.editor().highlight(
            preview
                .highlights
                .into_iter()
                .map(|range| (range, color))
                .collect(),
            cx,
        );
        self.editor().select(range.start..range.start, cx);
        if let Some(preview) = self.chat.as_mut().and_then(|run| run.preview.as_mut()) {
            preview.range = range.clone();
        }
        if from_agent {
            self.focus_editor(window, cx);
        }
        // The bubble follows the change once it's scrolled into view.
        cx.on_next_frame(window, move |this, window, cx| {
            let anchor = this.agent_anchor(window);
            if let Some(run) = this
                .chat
                .as_mut()
                .filter(|run| !run.agent.as_ref().is_some_and(|agent| agent.moved))
            {
                run.anchor = anchor;
                cx.notify();
            }
        });
        cx.notify();
        Ok(())
    }

    /// Accept or reject the agent's edit on screen. Returns false when the
    /// preview isn't an agent edit, for the caller to handle.
    pub(in crate::workspace) fn settle_agent_edit(
        &mut self,
        accept: bool,
        window: Option<&mut Window>,
        cx: &mut Context<Self>,
    ) -> bool {
        let mut window = window;
        // The edit may wait in another tab: settle it there. Rejecting
        // shows it, since its undo needs the window; accepting (e.g. as the
        // palette opens) doesn't move the user.
        let shown = self.active;
        let elsewhere = self
            .agent_edit_elsewhere()
            .and_then(|path| self.tab_for(&path));
        let mut quietly = false;
        match (elsewhere, window.as_deref_mut()) {
            (Some(ix), Some(window)) => self.activate(ix, window, cx),
            (Some(ix), None) => {
                self.active = ix;
                quietly = true;
            }
            _ => {}
        }
        let settled = self.settle_agent_edit_here(accept, window, cx);
        if quietly && shown < self.tabs.len() {
            self.active = shown;
        }
        settled
    }

    fn settle_agent_edit_here(
        &mut self,
        accept: bool,
        mut window: Option<&mut Window>,
        cx: &mut Context<Self>,
    ) -> bool {
        // The edit as proposed, still in the buffer, for its diff.
        let proposed = self.editor().text(cx);
        let Some(run) = self.chat.as_mut() else {
            return false;
        };
        let id = run.id;
        let Some(agent) = run.agent.as_mut() else {
            return false;
        };
        let pending = match agent.pending.take() {
            Some(Pending::Edit(pending)) => pending,
            other => {
                agent.pending = other;
                return false;
            }
        };
        let preview = run.preview.take();
        if let Some(agent) = run.agent.as_mut() {
            let path = match &run.bubble {
                Bubble::Preview { message, .. } => message.clone(),
                _ => pending.path.to_string_lossy().into_owned(),
            };
            agent.entries.push(ChatEntry::Edit {
                path,
                accepted: accept,
                diff: jig_kit::diff::edit_lines(&pending.before, &proposed),
            });
            agent.scroll.scroll_to_bottom();
            run.bubble = Bubble::Running {
                label: agent.label.clone(),
                started: agent.started,
                context: agent.context.clone(),
                step: agent.step.clone(),
            };
            agent.step.set("Working".into());
            if accept {
                agent.touched.push(pending.path.clone());
            }
        }
        self.editor().clear_highlights(cx);
        self.editor().set_readonly(false, cx);
        let mut note = None;
        if accept {
            // The agent writes the same text to disk, or leaves it to Jig.
            let text = self.editor().text(cx);
            if pending.jig_writes
                && let Err(error) = std::fs::write(&pending.path, &text)
            {
                note = Some(format!("Jig couldn't write the file: {error}"));
            }
            let tab = self.tab_mut();
            tab.document.saved_text = text;
            tab.dirty = false;
        } else if let Some(window) = window.as_deref_mut() {
            // Put the old text back over the new, rather than undo: undo
            // finds nothing when no key has gone through the editor since
            // the edit, as when Reject is clicked.
            if let Some(preview) = preview {
                let now = self.editor().text(cx);
                let start = preview.range.start;
                let old = (pending.before.len() + preview.range.len())
                    .checked_sub(now.len())
                    .and_then(|old_len| pending.before.get(start..start + old_len));
                if let Some(old) = old {
                    self.editor().apply_edit(preview.range, old, window, cx);
                    self.editor().select(start..start, cx);
                }
            }
            if pending.created {
                let _ = std::fs::remove_file(&pending.path);
                let run = self.chat.take();
                let tab = self.tab().id();
                self.remove_tab(tab, window, cx);
                self.chat = run;
                self.refresh_tree(cx);
            }
        }
        let written = note.is_none();
        self.answer_edit(&pending.id, accept && written, note.as_deref(), cx);
        // The agent may have asked for more at once, e.g. several edits, and
        // sends nothing else until those are answered too: show the next.
        if let Some(window) = window {
            self.show_next_request(id, window, cx);
        }
        cx.notify();
        true
    }

    pub(super) fn answer_edit(
        &self,
        edit_id: &str,
        accept: bool,
        note: Option<&str>,
        cx: &mut Context<Self>,
    ) {
        let session = self
            .chat
            .as_ref()
            .and_then(|run| run.agent.as_ref())
            .and_then(|agent| agent.session.clone());
        let (edit_id, note) = (edit_id.to_string(), note.map(str::to_string));
        if let Some(session) = session {
            cx.background_executor()
                .spawn(async move {
                    let _ = session.reply(&edit_id, accept, note.as_deref());
                })
                .detach();
        }
    }

    /// If the tab at `ix` has no unsaved changes but its file changed on
    /// disk, show the file's new text.
    pub(super) fn catch_up_with_disk(
        &mut self,
        ix: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let tab = &mut self.tabs[ix];
        let Some(path) = tab.document.path.clone() else {
            return;
        };
        let Ok(disk) = std::fs::read_to_string(&path) else {
            return;
        };
        let text = tab.editor.text(cx);
        if tab.dirty || disk == text {
            return;
        }
        let (range, replacement) = jig_kit::diff::changed_range(&text, &disk);
        tab.editor.apply_edit(range, replacement, window, cx);
        tab.document.saved_text = disk;
        tab.dirty = false;
    }
}
