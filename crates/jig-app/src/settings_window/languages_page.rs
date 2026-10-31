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
use crate::lsp;
use jig_debuggers::{self as debuggers, Debugger};
use jig_formatters::{self as formatters, Formatter};
use jig_languages::{self as languages, Language, Server, bundled};
use jig_settings as settings;

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

/// What formats `language`, as things stand.
fn formatter_note(language: &Language, cx: &App) -> String {
    let formatting = formatters::registry();
    let chosen = settings::get(cx)
        .formatting
        .chosen(language.name)
        .and_then(|key| formatting.get(key));
    if let Some(formatter) = chosen {
        return match formatter.program(Path::new("/")) {
            Some(_) => format!("{}, ahead of its language server.", formatter.name),
            None => format!(
                "{} once it's installed; until then, its language server.",
                formatter.name
            ),
        };
    }
    match formatting.for_language(language.name, Path::new("/")) {
        Some(formatter) => format!(
            "Its language server, or {} when it doesn't.",
            formatter.name
        ),
        None => match formatting.knows(language.name) {
            Some(formatter) => format!(
                "Its language server, or {} once it's installed.",
                formatter.name
            ),
            None => "Its language server, if it formats.".into(),
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

fn sentence(labels: &[&str]) -> String {
    match labels {
        [] => String::new(),
        [one] => one.to_string(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

/// What a library row says a language comes with: its server, formatters
/// and debugger, by name.
fn tools_line(language: &Language) -> String {
    let mut tools: Vec<String> = Vec::new();
    if let Some(server) = lsp::server_for(language.name).filter(|s| !s.commands.is_empty()) {
        tools.push(server.name.clone());
    }
    tools.extend(
        formatters::registry()
            .serving(language.name)
            .map(|formatter| formatter.name.clone()),
    );
    if let Some(debugger) = debuggers::registry().for_language(language.name) {
        tools.push(format!("{} debugger", debugger.name));
    }
    if tools.is_empty() {
        "Highlighting only.".into()
    } else {
        tools.join(" · ")
    }
}

/// Where an install is: its description, whether that's fine, and whether
/// it's still going.
fn progress(
    fetch: Option<&Fetch<SharedString>>,
    status: String,
    ready: bool,
) -> (String, bool, bool) {
    match fetch {
        Some(Fetch::Running(doing)) => (doing.to_string(), true, true),
        Some(Fetch::Done(done)) => (format!("{done} {status}"), ready, false),
        Some(Fetch::Failed(error)) => (error.to_string(), false, false),
        None => (status, ready, false),
    }
}

/// What a tool can do: be installed, and be taken out again.
struct Manage {
    id: String,
    busy: bool,
    installable: bool,
    ready: bool,
    removable: bool,
    install: Rc<dyn Fn(&mut App)>,
    uninstall: Rc<dyn Fn(&mut App)>,
}

impl Manage {
    /// Install when it's missing and Jig can fetch it; once Jig has, a
    /// menu to install it again or move it to the Trash. A copy installed
    /// some other way is left alone.
    fn render(&self) -> Div {
        let Manage {
            id,
            busy,
            installable,
            ready,
            removable,
            ..
        } = self;
        let (busy, installable, ready, removable) = (*busy, *installable, *ready, *removable);
        let install = self.install.clone();
        let uninstall = self.uninstall.clone();
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
            .when(removable, |row| {
                row.child(
                    Button::new(SharedString::from(format!("manage-{id}")))
                        .icon(IconName::Ellipsis)
                        .ghost()
                        .small()
                        .disabled(busy)
                        .dropdown_menu_with_anchor(Anchor::TopRight, move |menu, _, _| {
                            let (install, uninstall) = (install.clone(), uninstall.clone());
                            menu.when(installable, |menu| {
                                menu.item(
                                    PopupMenuItem::new("Reinstall")
                                        .on_click(move |_, _, cx| install(cx)),
                                )
                            })
                            .item(
                                PopupMenuItem::new("Uninstall")
                                    .on_click(move |_, _, cx| uninstall(cx)),
                            )
                        }),
                )
            })
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
        let key = format!("server-{}", server.name);
        let (description, ok, busy) = progress(self.installs.get(&key), status, ready);
        let manage = Manage {
            id: key.clone(),
            busy,
            installable: server.installable(),
            ready,
            removable: lsp::install::installed_by_jig(server),
            install: self.job_starter(
                &key,
                "Installing…",
                move || lsp::install::install(server).map(|()| "Installed."),
                lsp::servers_changed,
                cx,
            ),
            uninstall: self.job_starter(
                &key,
                "Moving to the Trash…",
                move || lsp::install::uninstall(server).map(|()| "Moved to the Trash."),
                lsp::servers_changed,
                cx,
            ),
        };
        let id = server.name.as_str();
        Row::new(key.clone(), id)
            .description(description)
            .warning(!ok)
            .keywords(["language server", "lsp", "install", "uninstall", id])
            .control(move |_, _| manage.render())
    }

    /// Which formatter the language uses, each one it could with Install
    /// and Uninstall, and formatting on save.
    fn formatting_section(&self, language: &'static Language, cx: &Context<Self>) -> Section {
        let name = language.name;
        let registry = formatters::registry();
        let serving: Vec<Arc<Formatter>> = registry.serving(name).cloned().collect();
        let mut section = Section::titled("Formatting");
        if !serving.is_empty() {
            let mut options: Vec<(SharedString, SharedString)> =
                vec![("".into(), "Language server".into())];
            options.extend(
                serving
                    .iter()
                    .map(|f| (f.key.clone().into(), f.name.clone().into())),
            );
            section = section.row(
                Row::new(format!("{name}-formatter"), "Formatter")
                    .description(formatter_note(language, cx))
                    .keywords(["format", "formatter", name])
                    .control(ui::dropdown(
                        format!("{name}-formatter"),
                        options,
                        move |cx| {
                            settings::get(cx)
                                .formatting
                                .chosen(name)
                                .unwrap_or_default()
                                .to_string()
                                .into()
                        },
                        move |key, cx| {
                            let key = (!key.is_empty()).then_some(key.as_ref());
                            settings::update(cx, |s| s.formatting.choose(name, key))
                        },
                    )),
            );
            section = section.rows(serving.into_iter().map(|f| self.formatter_row(f, cx)));
        }
        section.row(
            Row::new(format!("{name}-format"), "Format on save")
                .description_opt(serving_note(&registry, language, cx))
                .keywords(["format", "formatter", "save", name])
                .control(ui::switch(
                    format!("{name}-format"),
                    move |cx| settings::get(cx).formatting.on_save(name),
                    move |on, cx| settings::update(cx, |s| s.formatting.set_on_save(name, on)),
                )),
        )
    }

    fn formatter_row(&self, formatter: Arc<Formatter>, cx: &Context<Self>) -> Row {
        let (status, ready) = formatter.status();
        let key = format!("formatter-{}", formatter.key);
        let (description, ok, busy) = progress(self.installs.get(&key), status, ready);
        let (install, uninstall) = (formatter.clone(), formatter.clone());
        let manage = Manage {
            id: key.clone(),
            busy,
            installable: formatter.installable(),
            ready,
            removable: formatter.installed_by_jig(),
            install: self.job_starter(
                &key,
                "Installing…",
                move || install.install().map(|()| "Installed."),
                |_| {},
                cx,
            ),
            uninstall: self.job_starter(
                &key,
                "Moving to the Trash…",
                move || uninstall.uninstall().map(|()| "Moved to the Trash."),
                |_| {},
                cx,
            ),
        };
        Row::new(key.clone(), formatter.name.clone())
            .description(description)
            .warning(!ok)
            .keywords(["format", "formatter", "install", "uninstall"])
            .control(move |_, _| manage.render())
    }

    /// A debugger's switch, where Jig found it or what's missing, and
    /// Install when Jig can fetch it; once it's there, a menu to install
    /// it again or move what Jig installed to the Trash.
    fn debugger_row(&self, debugger: Arc<Debugger>, cx: &Context<Self>) -> Row {
        let (status, ready) = debuggers::status(&debugger);
        let key = format!("debugger-{}", debugger.key);
        let (description, ok, busy) = progress(self.installs.get(&key), status, ready);
        // Turned on once it's there; off unless another copy is found.
        let settle = {
            let debugger = debugger.clone();
            move |cx: &mut App| {
                let (_, ready) = debuggers::status(&debugger);
                settings::update(cx, |s| s.debugging.set_enabled(&debugger.key, ready));
            }
        };
        let (install, uninstall) = (debugger.clone(), debugger.clone());
        let manage = Manage {
            id: key.clone(),
            busy,
            installable: debugger.installable(),
            ready,
            removable: !debuggers::installed_files(&debugger).is_empty(),
            install: self.job_starter(
                &key,
                "Installing…",
                move || debuggers::install(&install).map(|()| "Installed."),
                settle.clone(),
                cx,
            ),
            uninstall: self.job_starter(
                &key,
                "Moving to the Trash…",
                move || debuggers::uninstall(&uninstall).map(|()| "Moved to the Trash."),
                settle,
                cx,
            ),
        };
        let switch_key = debugger.key.clone();
        let (get, set) = (switch_key.clone(), switch_key);
        let switch = ui::switch(
            key.clone(),
            move |cx| settings::get(cx).debugging.is_enabled(&get),
            move |on, cx| settings::update(cx, |s| s.debugging.set_enabled(&set, on)),
        );
        Row::new(key, debugger.name.clone())
            .description(description)
            .warning(!ok)
            .keywords(["debug", "debugger", "breakpoint", "install", "uninstall"])
            .control(move |window, cx| manage.render().child(switch(window, cx)))
    }

    /// A click handler running `run` in the background under `key`, its
    /// progress shown on the tool's row, then `settle` with what changed.
    fn job_starter<R, S>(
        &self,
        key: &str,
        doing: &'static str,
        run: R,
        settle: S,
        cx: &Context<Self>,
    ) -> Rc<dyn Fn(&mut App)>
    where
        R: Fn() -> anyhow::Result<&'static str> + Clone + Send + 'static,
        S: Fn(&mut App) + Clone + 'static,
    {
        let this = cx.entity().downgrade();
        let key = key.to_string();
        Rc::new(move |cx: &mut App| {
            let (key, run, settle) = (key.clone(), run.clone(), settle.clone());
            this.update(cx, |this, cx| this.tool_job(key, doing, run, settle, cx))
                .ok();
        })
    }

    fn tool_job(
        &mut self,
        key: String,
        doing: &'static str,
        run: impl FnOnce() -> anyhow::Result<&'static str> + Send + 'static,
        settle: impl FnOnce(&mut App) + 'static,
        cx: &mut Context<Self>,
    ) {
        if matches!(self.installs.get(&key), Some(Fetch::Running(_))) {
            return;
        }
        self.installs
            .insert(key.clone(), Fetch::Running(doing.into()));
        cx.spawn(async move |this, cx| {
            let result = cx.background_executor().spawn(async move { run() }).await;
            this.update(cx, |this, cx| {
                let fetch = match result {
                    Ok(done) => Fetch::Done(done.into()),
                    Err(error) => Fetch::Failed(format!("{error:#}").into()),
                };
                settle(cx);
                this.installs.insert(key, fetch);
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
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
