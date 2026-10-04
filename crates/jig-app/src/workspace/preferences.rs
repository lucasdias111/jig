//! Applying the Settings window's choices to an open workspace, and the
//! AI providers file it links to.

use std::path::PathBuf;

use gpui_kit::component::input::EditorState;
use gpui_kit::*;
use jig_commands::Preset;
use jig_editor::EditorHandle as _;

use super::{EditModelConfig, Workspace};
use crate::settings::{self, EditorSettings, Settings};

/// `presets` without the ones hidden in Settings.
pub(super) fn visible_presets(mut presets: Vec<Preset>, settings: &Settings) -> Vec<Preset> {
    presets.retain(|preset| !settings.commands.is_hidden(&preset.name));
    presets
}

fn apply_editor(
    state: &Entity<EditorState>,
    editor: &EditorSettings,
    window: &mut Window,
    cx: &mut App,
) {
    state.update(cx, |state, cx| {
        state.set_line_number(editor.line_numbers, window, cx);
        state.set_soft_wrap(editor.soft_wrap, window, cx);
        state.set_indent_guides(editor.indent_guides, window, cx);
        state.set_show_whitespaces(editor.show_whitespace, window, cx);
    });
}

impl Workspace {
    /// Bring the workspace in line with changed settings, redoing only what
    /// the change touched.
    pub(super) fn apply_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let new = settings::get(cx);
        let old = std::mem::replace(&mut self.settings, new.clone());
        if new.editor != old.editor {
            for tab in &self.tabs {
                apply_editor(tab.editor.state(), &new.editor, window, cx);
            }
        }
        if new.languages != old.languages {
            for tab in &self.tabs {
                let language = tab.document.language(&new.languages);
                if tab.editor.language(cx) != language {
                    tab.editor
                        .state()
                        .update(cx, |state, cx| state.set_highlighter(language, cx));
                }
            }
        }
        if new.commands != old.commands {
            self.reload_presets(window, cx);
        }
        cx.notify();
    }

    pub(super) fn reload_provider(&mut self, cx: &App) {
        self.provider = crate::providers::build(cx);
    }

    fn config_path(&self) -> Result<PathBuf, String> {
        self.config_path
            .clone()
            .ok_or_else(|| "Couldn't find your home folder.".to_string())
    }

    /// Open the AI providers file in this window, creating it from the
    /// built-in providers first.
    pub(super) fn edit_model_config(
        &mut self,
        _: &EditModelConfig,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let path = match self.config_path() {
            Ok(path) => path,
            Err(error) => return self.show_error(&error, window, cx),
        };
        if let Err(error) = jig_ai::Config::ensure_user_file(&path) {
            return self.show_error(&format!("{error:#}"), window, cx);
        }
        self.open_file(&path, window, cx);
    }

    pub(super) fn is_config_file(&self) -> bool {
        let Some(path) = &self.document().path else {
            return false;
        };
        self.config_path.as_ref().is_some_and(|config| {
            std::fs::canonicalize(config).ok() == std::fs::canonicalize(path).ok()
        })
    }
}
