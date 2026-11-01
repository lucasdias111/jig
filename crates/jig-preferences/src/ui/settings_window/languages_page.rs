//! Languages, kept like Neovim's Mason: the languages in the list, each
//! opening a page with everything about it (highlighting, extensions, its
//! language server, formatters and debugger, each with Install and
//! Uninstall), and the Library of every language Jig bundles, to add more
//! from. The files that add tools of your own are at the foot of the list.

use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::{Disableable as _, IconName, Sizable as _, h_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::ui::{self, Row, Section};
use super::{
    Detail, EditDebuggers, EditFormatters, EditLanguageServers, Fetch, Nav, Page, SettingsWindow,
    language, send_to_workspace,
};
use jig_debuggers::{self as debuggers, Debugger};
use jig_formatters::{self as formatters, Formatter};
use jig_languages::{self as languages, Language, Server, bundled};
use jig_lsp::lsp;
use jig_settings as settings;

use crate::domain::wording::sentence;

mod parts;

use parts::*;

impl SettingsWindow {
    pub(super) fn languages_sections(&self, cx: &Context<Self>) -> Vec<Section> {
        let settings = settings::get(cx);
        let registry = debuggers::registry();
        let added = bundled()
            .iter()
            .enumerate()
            .filter(|(_, language)| settings.languages.is_added(language.name));
        let rows = added.map(|(ix, language)| {
            let shown_extensions = extensions(language, cx)
                .split([',', ' '])
                .filter(|ext| !ext.is_empty())
                .map(|ext| format!(".{}", ext.trim_start_matches('.')))
                .collect::<Vec<_>>()
                .join(" ");
            let mut summary: Vec<&str> = Vec::new();
            if settings.languages.is_off(language.name) {
                summary.push("Plain text");
            }
            if settings.formatting.on_save(language.name) {
                summary.push("Formats on save");
            }
            if registry
                .for_language(language.name)
                .is_some_and(|debugger| settings.debugging.is_enabled(&debugger.key))
            {
                summary.push("Debugging");
            }
            Row::new(language.name, language.label)
                .description_opt((!summary.is_empty()).then(|| summary.join(" · ")))
                .value(shown_extensions)
                .keywords([language.name])
                .opens(self.opener(Nav::detail(Page::Languages, Detail::Language(ix)), cx))
        });

        let missing = bundled()
            .iter()
            .filter(|language| !settings.languages.is_added(language.name))
            .count();
        let library = Row::new("library", "Add languages")
            .description(if missing == 0 {
                "Every language Jig bundles is in the list.".to_string()
            } else {
                format!(
                    "{missing} more in the library, with their language servers, formatters \
                     and debuggers."
                )
            })
            .keywords(["add", "language", "library", "install", "mason"])
            .opens(self.opener(Nav::detail(Page::Languages, Detail::Library), cx));

        let debuggers_error = registry.error.clone();
        let formatters_error = formatters::registry().error.clone();
        let servers_error = languages::user_error();
        vec![
            Section::new().rows(rows).row(library),
            Section::titled("Your own tools")
                .footer(
                    "Add a tool Jig doesn't list, or change one it does. A tool here with the \
                     same name as a built-in replaces it.",
                )
                .row(
                    Row::new("servers-file", "Language servers")
                        .description(servers_error.map_or_else(
                            || {
                                "Any Language Server Protocol server, in \
                                 language-servers.toml. Restart Jig after changing it."
                                    .to_string()
                            },
                            |error| format!("{error}. Restart Jig after fixing it."),
                        ))
                        .warning(servers_error.is_some())
                        .keywords(["language server", "lsp", "server", "toml", "file"])
                        .control(|_, _| {
                            ui::button("edit-servers", "Edit…").on_click(|_, _, cx| {
                                send_to_workspace(Box::new(EditLanguageServers), cx)
                            })
                        }),
                )
                .row(
                    Row::new("formatters-file", "Formatters")
                        .description_opt(formatters_error.clone().or(Some(
                            "Any program formatting stdin to stdout, in formatters.toml.".into(),
                        )))
                        .warning(formatters_error.is_some())
                        .keywords(["format", "formatter", "toml", "file"])
                        .control(|_, _| {
                            ui::button("edit-formatters", "Edit…").on_click(|_, _, cx| {
                                send_to_workspace(Box::new(EditFormatters), cx)
                            })
                        }),
                )
                .row(
                    Row::new("debuggers-file", "Debuggers")
                        .description_opt(debuggers_error.clone().or(Some(
                            "Any Debug Adapter Protocol debugger, in debuggers.toml.".into(),
                        )))
                        .warning(debuggers_error.is_some())
                        .keywords(["debug", "debugger", "dap", "toml", "file"])
                        .control(|_, _| {
                            ui::button("edit-debuggers", "Edit…")
                                .on_click(|_, _, cx| send_to_workspace(Box::new(EditDebuggers), cx))
                        }),
                ),
        ]
    }

    /// Every language Jig bundles, the ones in the list first, each with
    /// the tools it comes with and Add or Remove. Searching finds a
    /// language here by its name, extensions or tools.
    pub(super) fn library_sections(&self, cx: &Context<Self>) -> Vec<Section> {
        let settings = settings::get(cx);
        let mut sorted: Vec<(usize, &'static Language)> = bundled().iter().enumerate().collect();
        sorted.sort_by_key(|(_, language)| language.label.to_lowercase());
        let row = |(ix, language): (usize, &'static Language)| {
            let added = settings.languages.is_added(language.name);
            let name = language.name;
            let this = cx.entity().downgrade();
            let mut keywords: Vec<SharedString> =
                vec![name.into(), "add".into(), "language".into()];
            keywords.extend(
                language
                    .extensions
                    .iter()
                    .chain(&language.filenames)
                    .map(|ext| SharedString::from(*ext)),
            );
            Row::new(format!("library-{name}"), language.label)
                .description(tools_line(language))
                .keywords(keywords)
                .opens(self.opener(Nav::detail(Page::Languages, Detail::Language(ix)), cx))
                .control(move |_, _| {
                    let this = this.clone();
                    let label = if added { "Remove" } else { "Add" };
                    ui::button(format!("library-{name}"), label).on_click(move |_, _, cx| {
                        settings::update(cx, |s| s.languages.set_added(name, !added));
                        this.update(cx, |_, cx| cx.notify()).ok();
                    })
                })
        };
        let (added, rest): (Vec<_>, Vec<_>) = sorted
            .into_iter()
            .partition(|(_, language)| settings.languages.is_added(language.name));
        vec![
            Section::titled("In your list").rows(added.into_iter().map(row)),
            Section::titled("More languages")
                .footer(
                    "Grammars are part of Jig, so adding one is instant; its language server, \
                     formatters and debugger install from its page.",
                )
                .rows(rest.into_iter().map(row)),
        ]
    }

    pub(super) fn language_sections(&self, ix: usize, cx: &Context<Self>) -> Vec<Section> {
        let language = language(ix);
        let name = language.name;
        let label = language.label;
        let built_in = languages::format_extensions(&language.extensions);
        let added = settings::get(cx).languages.is_added(name);
        let mut sections = Vec::new();
        if !added {
            sections.push(
                Section::new().row(
                    Row::new(format!("{name}-add"), format!("Add {label}"))
                        .description(format!(
                            "{label} files open as plain text until it's in the list."
                        ))
                        .keywords(["add", "language", name])
                        .control(move |_, _| {
                            ui::button(format!("{name}-add"), "Add").on_click(move |_, _, cx| {
                                settings::update(cx, |s| s.languages.set_added(name, true))
                            })
                        }),
                ),
            );
        }
        sections.push(
            Section::new()
                .row(
                    Row::new(format!("{name}-highlight"), "Syntax highlighting")
                        .description(format!("Off, {label} files open as plain text."))
                        .keywords(["highlight", "syntax", "plain text", name])
                        .control(ui::switch(
                            format!("{name}-highlight"),
                            move |cx| !settings::get(cx).languages.is_off(name),
                            move |on, cx| settings::update(cx, |s| s.languages.set_off(name, !on)),
                        )),
                )
                .row(
                    Row::new(format!("{name}-extensions"), "File extensions")
                        .description(format!(
                            "Separated by commas. {built_in} by default; where two \
                             languages claim one, the higher in the list wins."
                        ))
                        .keywords(["extension", "file type", "associate", name])
                        .control(ui::text_input(
                            format!("{name}-extensions"),
                            move |cx| extensions(language, cx).into(),
                            move |text, cx| {
                                let built_in = languages::format_extensions(&language.extensions);
                                settings::update(cx, |s| {
                                    s.languages.set_extensions(
                                        name,
                                        Some(text.to_string()),
                                        &built_in,
                                    )
                                })
                            },
                        )),
                ),
        );
        sections.push(self.server_section(name, cx));
        sections.push(self.formatting_section(language, cx));
        if let Some(debugger) = debuggers::registry().for_language(name) {
            let shared: Vec<String> = debugger
                .languages
                .iter()
                .filter(|other| *other != name)
                .cloned()
                .collect();
            let mut section = Section::titled("Debugging").row(self.debugger_row(debugger, cx));
            if !shared.is_empty() {
                section = section.footer(format!(
                    "Breakpoints and ⌃D. The same debugger serves {}.",
                    names(&shared)
                ));
            } else {
                section = section.footer("Breakpoints and ⌃D.");
            }
            sections.push(section);
        }
        if added {
            let this = cx.entity().downgrade();
            sections.push(
                Section::new().row(
                    Row::new(format!("{name}-remove"), format!("Remove {label}"))
                        .description(format!(
                            "Takes it off the list; {label} files open as plain text. Its tools \
                             stay installed."
                        ))
                        .keywords(["remove", "language", name])
                        .control(move |_, _| {
                            let this = this.clone();
                            ui::button(format!("{name}-remove"), "Remove").on_click(
                                move |_, window, cx| {
                                    settings::update(cx, |s| s.languages.set_added(name, false));
                                    this.update(cx, |this, cx| {
                                        this.go(Nav::page(Page::Languages), window, cx)
                                    })
                                    .ok();
                                },
                            )
                        }),
                ),
            );
        }
        sections
    }
}

/// Under Format on save: what does it, when there's no Formatter row to
/// say so.
fn serving_note(registry: &formatters::Registry, language: &Language, cx: &App) -> Option<String> {
    registry
        .knows(language.name)
        .is_none()
        .then(|| formatter_note(language, cx))
}

mod tools;

#[cfg(test)]
mod tests;
