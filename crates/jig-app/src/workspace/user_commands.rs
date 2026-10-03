//! Adding and editing the user's own preset commands.

use std::path::PathBuf;
use std::rc::Rc;

use gpui_kit::*;
use jig_commands::{NewCommandEvent, NewCommandForm, Scope, presets};
use jig_editor::EditorHandle;

use super::{AddCommand, EditCommands, EditProjectRules, Workspace};
use crate::project;

pub(super) struct OpenForm {
    pub(super) view: Entity<NewCommandForm>,
    _events: Subscription,
}

impl Workspace {
    fn commands_path(&self) -> Result<PathBuf, String> {
        self.commands_path
            .clone()
            .ok_or_else(|| "Couldn't find your home folder.".to_string())
    }

    pub(super) fn add_command(
        &mut self,
        _: &AddCommand,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_add_command(None, window, cx);
    }

    /// Show the new-command form, with `prompt` pre-filled when it comes
    /// from text typed into the command input.
    pub(super) fn open_add_command(
        &mut self,
        prompt: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.new_command.is_some() {
            return;
        }
        self.palette = None;
        let view = cx.new(|cx| NewCommandForm::new(prompt, Scope::Selection, window, cx));
        let events = cx.subscribe_in(
            &view,
            window,
            |this, view, event: &NewCommandEvent, window, cx| match event {
                NewCommandEvent::Cancel => this.close_add_command(window, cx),
                NewCommandEvent::Save(preset) => {
                    let saved = this.commands_path().and_then(|path| {
                        presets::add_user_preset(&path, preset)
                            .map_err(|error| format!("{error:#}"))
                    });
                    match saved {
                        Ok(()) => {
                            this.reload_presets(window, cx);
                            this.close_add_command(window, cx);
                            this.show_note(
                                format!("Added “{}”. Run it with ⌘K.", preset.name),
                                window,
                                cx,
                            );
                        }
                        Err(error) => view.update(cx, |form, cx| form.set_error(error, cx)),
                    }
                }
            },
        );
        self.new_command = Some(OpenForm {
            view,
            _events: events,
        });
        cx.notify();
    }

    fn close_add_command(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.new_command.take().is_some() {
            self.editor().focus(window, cx);
            cx.notify();
        }
    }

    /// Open the user's commands file in this window, creating it first.
    pub(super) fn edit_commands(
        &mut self,
        _: &EditCommands,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let path = match self.commands_path() {
            Ok(path) => path,
            Err(error) => return self.show_error(&error, window, cx),
        };
        if let Err(error) = presets::ensure_user_file(&path) {
            return self.show_error(&format!("{error:#}"), window, cx);
        }
        self.open_file(&path, window, cx);
    }

    /// Open the `JIG.md` that applies to the current file, creating one at
    /// the project root if there is none.
    pub(super) fn edit_project_rules(
        &mut self,
        _: &EditProjectRules,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(file) = self.document().path.clone() else {
            return self.show_error(
                "Save this file first, so Jig knows which project it belongs to.",
                window,
                cx,
            );
        };
        let path = project::rules_path(&file)
            .unwrap_or_else(|| project::root_for(&file).join(project::RULES_FILE));
        if !path.exists()
            && let Err(error) = std::fs::write(&path, project::RULES_TEMPLATE)
        {
            return self.show_error(
                &format!("Couldn't create {}: {error}", path.display()),
                window,
                cx,
            );
        }
        self.open_file(&path, window, cx);
    }

    /// Re-read the presets, e.g. after the commands file was saved.
    pub(super) fn reload_presets(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match presets::load(self.commands_path.as_deref()) {
            Ok(presets) => self.presets = Rc::new(presets),
            Err(error) => self.show_error(
                &format!("Your commands weren't reloaded. {error:#}"),
                window,
                cx,
            ),
        }
    }

    pub(super) fn is_commands_file(&self) -> bool {
        let Some(path) = &self.document().path else {
            return false;
        };
        self.commands_path.as_ref().is_some_and(|user| {
            std::fs::canonicalize(user).ok() == std::fs::canonicalize(path).ok()
        })
    }
}
