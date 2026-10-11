//! Languages: one row per added language, summing up how it's set, each
//! opening a page with everything about it (highlighting, extensions,
//! formatting on save, its debugger), and a menu to add more from the ones
//! Jig bundles. The files that add formatters and debuggers are at the
//! foot of the list.

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
use crate::languages::{self, Language, bundled};
use crate::lsp::{self, Server};
use crate::settings;
use crate::workspace::{EditDebuggers, EditFormatters, EditLanguageServers};

/// A language's extensions as Settings shows and takes them: the user's,
/// or the built-in ones.
fn extensions(language: &Language, cx: &App) -> String {
    settings::get(cx)
        .languages
        .extensions
        .get(language.name)
        .cloned()
        .unwrap_or_else(|| languages::format_extensions(&language.extensions))
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
            bundled()
                .iter()
                .find(|language| language.name == name)
                .map_or(name.as_str(), |language| language.label)
        })
        .collect();
    sentence(&labels)
}

/// The languages' labels in a sentence.
fn names_of(languages: &[&Language]) -> String {
    let labels: Vec<&str> = languages.iter().map(|language| language.label).collect();
    sentence(&labels)
}

fn sentence(labels: &[&str]) -> String {
    match labels {
        [] => String::new(),
        [one] => one.to_string(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

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

        let debuggers_error = registry.error.clone();
        let formatters_error = formatters::registry().error.clone();
        vec![
            Section::new().rows(rows).row(self.add_language_row(cx)),
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
            Section::titled("Language servers").row({
                let error = languages::user_error();
                Row::new("servers-file", "Add or change language servers")
                    .description(error.map_or_else(
                        || {
                            "Any Language Server Protocol server, in language-servers.toml. \
                             Restart Jig after changing it."
                                .to_string()
                        },
                        |error| format!("{error}. Restart Jig after fixing it."),
                    ))
                    .warning(error.is_some())
                    .keywords(["language server", "lsp", "server", "toml", "file"])
                    .control(|_, _| {
                        ui::button("edit-servers", "Edit…").on_click(|_, _, cx| {
                            send_to_workspace(Box::new(EditLanguageServers), cx)
                        })
                    })
            }),
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

    /// A menu of the bundled languages not in the list yet. Searching for
    /// one finds this row.
    fn add_language_row(&self, cx: &Context<Self>) -> Row {
        let settings = settings::get(cx);
        let missing: Vec<&'static Language> = bundled()
            .iter()
            .filter(|language| !settings.languages.is_added(language.name))
            .collect();
        let description = if missing.is_empty() {
            "Every language Jig bundles is in the list.".to_string()
        } else {
            format!(
                "{} more, such as {}.",
                missing.len(),
                names_of(&missing[..3.min(missing.len())])
            )
        };
        let mut keywords = vec!["add", "language"];
        keywords.extend(
            missing
                .iter()
                .flat_map(|language| [language.name, language.label]),
        );
        let this = cx.entity().downgrade();
        Row::new("add-language", "Add a language")
            .description(description)
            .keywords(keywords)
            .control(move |_, cx| {
                let this = this.clone();
                let none_left = bundled()
                    .iter()
                    .all(|language| settings::get(cx).languages.is_added(language.name));
                ui::button("add-language", "Add…")
                    .disabled(none_left)
                    .dropdown_menu_with_anchor(Anchor::TopRight, move |mut menu, _, cx| {
                        menu = menu.scrollable(true).max_h(px(320.));
                        let added = settings::get(cx).languages.added.clone();
                        for (ix, language) in bundled().iter().enumerate() {
                            if added.iter().any(|name| name == language.name) {
                                continue;
                            }
                            let this = this.clone();
                            menu = menu.item(PopupMenuItem::new(language.label).on_click(
                                move |_, window, cx| {
                                    settings::update(cx, |s| {
                                        s.languages.set_added(language.name, true)
                                    });
                                    this.update(cx, |this, cx| {
                                        this.go(
                                            Nav::detail(Page::Languages, Detail::Language(ix)),
                                            window,
                                            cx,
                                        )
                                    })
                                    .ok();
                                },
                            ));
                        }
                        menu
                    })
            })
    }

    pub(super) fn language_sections(&self, ix: usize, cx: &Context<Self>) -> Vec<Section> {
        let language = language(ix);
        let name = language.name;
        let label = language.label;
        let built_in = languages::format_extensions(&language.extensions);
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
            self.server_section(name, cx),
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
        let this = cx.entity().downgrade();
        sections.push(
            Section::new().row(
                Row::new(format!("{name}-remove"), format!("Remove {label}"))
                    .description(format!(
                        "Takes it off the list; {label} files open as plain text."
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
        sections
    }

    /// The language's server: the program Jig runs, or how to get it, with
    /// Install when Jig can fetch it and Uninstall once it has.
    fn server_section(&self, language: &'static str, cx: &Context<Self>) -> Section {
        let section = Section::titled("Language server");
        let Some(server) = lsp::server_for(language) else {
            return section.footer(
                "Jig knows no language server for it, so go to definition and completion use \
                 words from the file.",
            );
        };
        let shared = names(
            &server
                .languages
                .iter()
                .filter(|other| *other != language)
                .cloned()
                .collect::<Vec<_>>(),
        );
        let footer = if shared.is_empty() {
            "Go to definition, completion, problems, hover and rename.".to_string()
        } else {
            format!(
                "Go to definition, completion, problems, hover and rename. It serves {shared} too."
            )
        };
        section.footer(footer).row(self.server_row(server, cx))
    }

    fn server_row(&self, server: &'static Server, cx: &Context<Self>) -> Row {
        let (status, ready) = lsp::install::status(server);
        let job = format!("server-{}", server.name);
        let fetch = self.installs.get(&job);
        let (description, ok) = match fetch {
            Some(Fetch::Running(doing)) => (doing.to_string(), true),
            Some(Fetch::Done(done)) => (format!("{done} {status}"), ready),
            Some(Fetch::Failed(error)) => (error.to_string(), false),
            None => (status, ready),
        };
        let busy = matches!(fetch, Some(Fetch::Running(_)));
        let installable = server.installable();
        let removable = lsp::install::installed_by_jig(server);
        let this = cx.entity().downgrade();
        let id = server.name.as_str();
        Row::new(format!("server-{id}"), id)
            .description(description)
            .warning(!ok)
            .keywords(["language server", "lsp", "install", "uninstall", id])
            .control(move |_, _| {
                let job = |doing: &'static str, run: fn(&Server) -> anyhow::Result<()>, done| {
                    let this = this.clone();
                    move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                        this.update(cx, |this, cx| this.server_job(server, doing, run, done, cx))
                            .ok();
                    }
                };
                h_flex()
                    .gap_2()
                    .items_center()
                    .when(installable && !ready && !removable, |row| {
                        row.child(
                            ui::button(format!("install-{id}"), "Install")
                                .disabled(busy)
                                .on_click(job("Installing…", lsp::install::install, "Installed.")),
                        )
                    })
                    .when(removable, |row| {
                        row.child(
                            ui::button(format!("uninstall-{id}"), "Uninstall")
                                .disabled(busy)
                                .on_click(job(
                                    "Moving to the Trash…",
                                    lsp::install::uninstall,
                                    "Moved to the Trash.",
                                )),
                        )
                    })
            })
    }

    /// Install or uninstall `server` in the background, then have open
    /// files look for it again.
    fn server_job(
        &mut self,
        server: &'static Server,
        doing: &'static str,
        run: fn(&Server) -> anyhow::Result<()>,
        done: &'static str,
        cx: &mut Context<Self>,
    ) {
        let key = format!("server-{}", server.name);
        if matches!(self.installs.get(&key), Some(Fetch::Running(_))) {
            return;
        }
        self.installs
            .insert(key.clone(), Fetch::Running(doing.into()));
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { run(server) })
                .await;
            this.update(cx, |this, cx| {
                let fetch = match result {
                    Ok(()) => Fetch::Done(done.into()),
                    Err(error) => Fetch::Failed(format!("{error:#}").into()),
                };
                lsp::servers_changed(cx);
                this.installs.insert(key, fetch);
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
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
        assert_eq!(names(&["cobol".into()]), "cobol");
    }
}
