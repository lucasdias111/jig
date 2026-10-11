//! Jigs: add one, open the jigs file, and turn each jig, the user's and the
//! built-in ones, on or off in the ⌘K input.
//!
//! Add Jig… opens the form over Settings itself, so Settings stays in front.

use gpui_kit::component::button::ButtonVariants as _;
use gpui_kit::component::h_flex;
use gpui_kit::*;
use jig_commands::{NewCommandEvent, NewCommandForm, Preset, Scope, presets};

use super::ui::{self, Row, Section};
use super::{SettingsWindow, send_to_every_workspace, send_to_workspace};
use crate::settings;
use crate::workspace::{EditCommands, ReloadJigs};

/// Longest prompt shown under a jig's name.
const MAX_PROMPT_CHARS: usize = 90;

/// A jig as the page lists it.
#[derive(Clone)]
pub(super) struct CommandRow {
    name: SharedString,
    detail: SharedString,
}

impl CommandRow {
    fn new(preset: &Preset) -> Self {
        let prompt = preset
            .prompt
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        let prompt = if prompt.chars().count() > MAX_PROMPT_CHARS {
            let cut: String = prompt.chars().take(MAX_PROMPT_CHARS).collect();
            format!("{}…", cut.trim_end())
        } else {
            prompt
        };
        let scope = preset.scope.label();
        let mut scope = scope.chars();
        let scope: String = scope
            .next()
            .map(|first| first.to_uppercase().chain(scope).collect())
            .unwrap_or_default();
        Self {
            name: preset.name.clone().into(),
            detail: format!("{scope} · {prompt}").into(),
        }
    }

    fn row(&self) -> Row {
        let (get, set) = (self.name.clone(), self.name.clone());
        Row::new(format!("jig-{}", self.name), self.name.clone())
            .description(self.detail.clone())
            .keywords(["jig", "command"])
            .control(ui::switch(
                format!("jig-{}", self.name),
                move |cx| !settings::get(cx).commands.is_hidden(&get),
                move |shown, cx| settings::update(cx, |s| s.commands.set_hidden(&set, !shown)),
            ))
    }
}

/// The Add Jig form, while it's open.
pub(super) struct NewJig {
    pub(super) view: Entity<NewCommandForm>,
    _events: Subscription,
}

impl SettingsWindow {
    pub(super) fn open_new_jig(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.new_jig.is_some() {
            return;
        }
        let view = cx.new(|cx| NewCommandForm::new(None, Scope::Selection, window, cx));
        let events = cx.subscribe_in(
            &view,
            window,
            |this, view, event: &NewCommandEvent, _, cx| match event {
                NewCommandEvent::Cancel => this.close_new_jig(cx),
                NewCommandEvent::Save(preset) => {
                    let saved = presets::user_jigs_path()
                        .ok_or_else(|| "Couldn't find your home folder.".to_string())
                        .and_then(|path| {
                            presets::add_user_preset(&path, preset)
                                .map_err(|error| format!("{error:#}"))
                        });
                    match saved {
                        Ok(()) => {
                            this.reload_commands();
                            this.close_new_jig(cx);
                            send_to_every_workspace(|| Box::new(ReloadJigs), cx);
                        }
                        Err(error) => view.update(cx, |form, cx| form.set_error(error, cx)),
                    }
                }
            },
        );
        self.new_jig = Some(NewJig {
            view,
            _events: events,
        });
        cx.notify();
    }

    fn close_new_jig(&mut self, cx: &mut Context<Self>) {
        if self.new_jig.take().is_some() {
            cx.notify();
        }
    }

    pub(super) fn reload_commands(&mut self) {
        let path = presets::user_jigs_path();
        let user = path
            .as_deref()
            .filter(|path| path.exists())
            .map(|path| {
                std::fs::read_to_string(path)
                    .map_err(anyhow::Error::from)
                    .and_then(|source| presets::parse(&source))
            })
            .transpose();
        let user = match user {
            Ok(user) => {
                self.commands_error = None;
                user.unwrap_or_default()
            }
            Err(error) => {
                self.commands_error =
                    Some(format!("Your jigs file has a problem: {error:#}").into());
                Vec::new()
            }
        };
        let replaced = |preset: &Preset| {
            user.iter()
                .any(|mine| mine.name.eq_ignore_ascii_case(&preset.name))
        };
        self.built_in_commands = presets::defaults()
            .iter()
            .filter(|preset| !replaced(preset))
            .map(CommandRow::new)
            .collect();
        self.user_commands = user.iter().map(CommandRow::new).collect();
    }

    pub(super) fn jigs_sections(&self, cx: &Context<Self>) -> Vec<Section> {
        let error = self.commands_error.clone();
        let this = cx.entity().downgrade();
        let mut sections = vec![
            Section::new()
                .footer("Jigs turned off leave the ⌘K input but stay defined.")
                .row(
                    Row::new("add-jig", "Your jigs file")
                        .description_opt(error.clone().or(Some(
                            "Save a prompt as a jig, or write jigs in jigs.toml.".into(),
                        )))
                        .warning(error.is_some())
                        .keywords(["add", "new", "file", "toml", "jig"])
                        .control(move |_, _| {
                            let this = this.clone();
                            h_flex()
                                .gap_2()
                                .child(ui::button("edit-jigs", "Edit…").on_click(|_, _, cx| {
                                    send_to_workspace(Box::new(EditCommands), cx)
                                }))
                                .child(ui::button("add-jig", "Add Jig…").primary().on_click(
                                    move |_, window, cx| {
                                        this.update(cx, |this, cx| this.open_new_jig(window, cx))
                                            .ok();
                                    },
                                ))
                        }),
                ),
        ];
        if !self.user_commands.is_empty() {
            sections.push(
                Section::titled("Your Jigs").rows(self.user_commands.iter().map(CommandRow::row)),
            );
        }
        sections.push(
            Section::titled("Built-in").rows(self.built_in_commands.iter().map(CommandRow::row)),
        );
        sections
    }
}

#[cfg(test)]
mod tests {
    use jig_commands::Preset;

    use super::{CommandRow, MAX_PROMPT_CHARS};

    #[test]
    fn long_prompts_are_shortened_to_one_line() {
        let row = CommandRow::new(&Preset {
            name: "A".into(),
            prompt: format!("first line\n  second {}", "word ".repeat(40)),
            ..Default::default()
        });
        assert!(row.detail.starts_with("Selection · first line second word"));
        assert!(row.detail.ends_with('…'));
        assert!(row.detail.chars().count() < MAX_PROMPT_CHARS + 20);
    }
}
