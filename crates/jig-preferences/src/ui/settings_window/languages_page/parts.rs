//! What the Languages pages' rows share: how a language's extensions,
//! formatter and tools read, an install's progress, and Install and
//! Uninstall buttons.

use super::*;

/// A language's extensions as Settings shows and takes them: the user's,
/// or the built-in ones.
pub(super) fn extensions(language: &Language, cx: &App) -> String {
    settings::get(cx)
        .languages
        .extensions
        .get(language.name)
        .cloned()
        .unwrap_or_else(|| languages::format_extensions(&language.extensions))
}

/// What formats `language`, as things stand.
pub(super) fn formatter_note(language: &Language, cx: &App) -> String {
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
pub(super) fn names(languages: &[String]) -> String {
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

/// What a library row says a language comes with: its server, formatters
/// and debugger, by name.
pub(super) fn tools_line(language: &Language) -> String {
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
pub(super) fn progress(
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
pub(super) struct Manage {
    pub(super) id: String,
    pub(super) busy: bool,
    pub(super) installable: bool,
    pub(super) ready: bool,
    pub(super) removable: bool,
    pub(super) install: Rc<dyn Fn(&mut App)>,
    pub(super) uninstall: Rc<dyn Fn(&mut App)>,
}

impl Manage {
    /// Install when it's missing and Jig can fetch it; once Jig has, a
    /// menu to install it again or move it to the Trash. A copy installed
    /// some other way is left alone.
    pub(super) fn render(&self) -> Div {
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
