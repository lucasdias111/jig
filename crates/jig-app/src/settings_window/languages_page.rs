//! Languages: one row per bundled language, summing up how it's set, each
//! opening a page with everything about it (highlighting, extensions,
//! formatting on save, its debugger). The files that add formatters and
//! debuggers are at the foot of the list.

use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::{Disableable as _, IconName, Sizable as _, h_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::ui::{self, Row, Section};
use super::{Detail, Fetch, Nav, Page, SettingsWindow, language, send_to_workspace};
use crate::debuggers::{self, Debugger};
use crate::formatters;
use crate::languages::{self, BUNDLED, Language};
use crate::settings;
use crate::workspace::{EditDebuggers, EditFormatters};

/// A language's extensions as Settings shows and takes them: the user's,
/// or the built-in ones.
fn extensions(language: &Language, cx: &App) -> String {
    settings::get(cx)
        .languages
        .extensions
        .get(language.name)
        .cloned()
        .unwrap_or_else(|| languages::format_extensions(language.extensions))
}

/// What formats `language` when its language server doesn't.
fn formatter_note(language: &Language) -> String {
    let formatting = formatters::registry();
    match formatting.for_language(language.name, Path::new("/")) {
        Some(formatter) => format!("With its language server, or {}.", formatter.name),
        None => match formatting.knows(language.name) {
            Some(formatter) => format!(
                "With its language server, or {} once it's installed.",
                formatter.name
            ),
            None => "With its language server, if it formats.".into(),
        },
    }
}

/// "Rust" for one language, "TypeScript, TSX and JavaScript" for several.
fn names(languages: &[String]) -> String {
    let labels: Vec<&str> = languages
        .iter()
        .map(|name| {
            BUNDLED
                .iter()
                .find(|language| language.name == name)
                .map_or(name.as_str(), |language| language.label)
        })
        .collect();
    match labels.as_slice() {
        [] => String::new(),
        [one] => one.to_string(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

impl SettingsWindow {
    pub(super) fn languages_sections(&self, cx: &Context<Self>) -> Vec<Section> {
        let settings = settings::get(cx);
        let registry = debuggers::registry();
        let rows = BUNDLED.iter().enumerate().map(|(ix, language)| {
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

        let debuggers_error = registry.error.clone();
        let formatters_error = formatters::registry().error.clone();
        vec![
            Section::new().rows(rows),
            Section::titled("Debuggers")
                .footer(
                    "Debuggers turned on take breakpoints and debug with ⌃D. Each also shows \
                     on its languages' pages.",
                )
                .rows(
                    registry
                        .debuggers
                        .iter()
                        .map(|debugger| self.debugger_row(debugger.clone(), cx)),
                )
                .row(
                    Row::new("debuggers-file", "Add or change debuggers")
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
            Section::titled("Formatters").row(
                Row::new("formatters-file", "Add or change formatters")
                    .description_opt(
                        formatters_error.clone().or(Some(
                            "For files whose language server doesn't format, in formatters.toml."
                                .into(),
                        )),
                    )
                    .warning(formatters_error.is_some())
                    .keywords(["format", "formatter", "toml", "file"])
                    .control(|_, _| {
                        ui::button("edit-formatters", "Edit…")
                            .on_click(|_, _, cx| send_to_workspace(Box::new(EditFormatters), cx))
                    }),
            ),
        ]
    }

    pub(super) fn language_sections(&self, ix: usize, cx: &Context<Self>) -> Vec<Section> {
        let language = language(ix);
        let name = language.name;
        let label = language.label;
        let built_in = languages::format_extensions(language.extensions);
        let mut sections = vec![
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
                                let built_in = languages::format_extensions(language.extensions);
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
            Section::titled("Formatting").row(
                Row::new(format!("{name}-format"), "Format on save")
                    .description(formatter_note(language))
                    .keywords(["format", "formatter", "save", name])
                    .control(ui::switch(
                        format!("{name}-format"),
                        move |cx| settings::get(cx).formatting.on_save(name),
                        move |on, cx| settings::update(cx, |s| s.formatting.set_on_save(name, on)),
                    )),
            ),
        ];
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
        sections
    }

    /// A debugger's switch, where Jig found it or what's missing, and
    /// Install when Jig can fetch it; once it's there, a menu to install
    /// it again or move what Jig installed to the Trash.
    fn debugger_row(&self, debugger: Arc<Debugger>, cx: &Context<Self>) -> Row {
        let (status, ready) = debuggers::status(&debugger);
        let fetch = self.installs.get(&debugger.key);
        let (description, ok) = match fetch {
            Some(Fetch::Running(doing)) => (doing.to_string(), true),
            Some(Fetch::Done(done)) => (format!("{done} {status}"), ready),
            Some(Fetch::Failed(error)) => (error.to_string(), false),
            None => (status, ready),
        };
        let busy = matches!(fetch, Some(Fetch::Running(_)));
        let installable = debugger.installable();
        let removable = !debuggers::installed_files(&debugger).is_empty();
        let this = cx.entity().downgrade();
        let key = debugger.key.clone();
        let (get, set) = (key.clone(), key.clone());
        let switch = ui::switch(
            format!("debug-{key}"),
            move |cx| settings::get(cx).debugging.is_enabled(&get),
            move |on, cx| settings::update(cx, |s| s.debugging.set_enabled(&set, on)),
        );
        Row::new(format!("debug-{key}"), debugger.name.clone())
            .description(description)
            .warning(!ok)
            .keywords(["debug", "debugger", "breakpoint", "install", "uninstall"])
            .control(move |window, cx| {
                let this = this.clone();
                let debugger = debugger.clone();
                let id = debugger.key.clone();
                let install = {
                    let (this, debugger) = (this.clone(), debugger.clone());
                    Rc::new(move |cx: &mut App| {
                        this.update(cx, |this, cx| this.install(debugger.clone(), cx))
                            .ok();
                    })
                };
                h_flex()
                    .gap_2()
                    .items_center()
                    .when(installable && !ready && !removable, |row| {
                        let install = install.clone();
                        row.child(
                            ui::button(format!("install-{id}"), "Install")
                                .disabled(busy)
                                .on_click(move |_, _, cx| install(cx)),
                        )
                    })
                    .when((installable && ready) || removable, |row| {
                        let install = install.clone();
                        row.child(
                            Button::new(SharedString::from(format!("manage-{id}")))
                                .icon(IconName::Ellipsis)
                                .ghost()
                                .small()
                                .disabled(busy)
                                .dropdown_menu_with_anchor(Anchor::TopRight, move |menu, _, _| {
                                    let install = install.clone();
                                    let (this, debugger) = (this.clone(), debugger.clone());
                                    menu.when(installable, |menu| {
                                        menu.item(
                                            PopupMenuItem::new("Reinstall")
                                                .on_click(move |_, _, cx| install(cx)),
                                        )
                                    })
                                    .when(removable, |menu| {
                                        menu.item(PopupMenuItem::new("Uninstall").on_click(
                                            move |_, _, cx| {
                                                let debugger = debugger.clone();
                                                this.update(cx, |this, cx| {
                                                    this.uninstall(debugger, cx)
                                                })
                                                .ok();
                                            },
                                        ))
                                    })
                                }),
                        )
                    })
                    .child(switch(window, cx))
            })
    }

    /// Run the debugger's install command in the background; turn it on
    /// once it's there.
    fn install(&mut self, debugger: Arc<Debugger>, cx: &mut Context<Self>) {
        self.debugger_job(
            debugger,
            "Installing…",
            |debugger| debuggers::install(debugger).map(|()| "Installed."),
            cx,
        );
    }

    /// Move what Jig installed of the debugger to the Trash; turn it off
    /// unless another copy is still found.
    fn uninstall(&mut self, debugger: Arc<Debugger>, cx: &mut Context<Self>) {
        self.debugger_job(
            debugger,
            "Moving to the Trash…",
            |debugger| debuggers::uninstall(debugger).map(|()| "Moved to the Trash."),
            cx,
        );
    }

    fn debugger_job(
        &mut self,
        debugger: Arc<Debugger>,
        doing: &'static str,
        job: fn(&Debugger) -> anyhow::Result<&'static str>,
        cx: &mut Context<Self>,
    ) {
        if matches!(self.installs.get(&debugger.key), Some(Fetch::Running(_))) {
            return;
        }
        self.installs
            .insert(debugger.key.clone(), Fetch::Running(doing.into()));
        cx.spawn(async move |this, cx| {
            let working = debugger.clone();
            let result = cx
                .background_executor()
                .spawn(async move { job(&working) })
                .await;
            this.update(cx, |this, cx| {
                let fetch = match result {
                    Ok(done) => {
                        let (_, ready) = debuggers::status(&debugger);
                        settings::update(cx, |s| s.debugging.set_enabled(&debugger.key, ready));
                        Fetch::Done(done.into())
                    }
                    Err(error) => Fetch::Failed(format!("{error:#}").into()),
                };
                this.installs.insert(debugger.key.clone(), fetch);
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::names;

    #[test]
    fn names_languages_in_a_sentence() {
        assert_eq!(names(&["rust".into()]), "Rust");
        assert_eq!(
            names(&["typescript".into(), "tsx".into(), "javascript".into()]),
            "TypeScript, TSX and JavaScript"
        );
        assert_eq!(names(&["zig".into()]), "zig");
    }
}
