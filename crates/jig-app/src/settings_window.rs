//! The Settings window (⌘,): Appearance, Colors, Languages, Commands and
//! Model.

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::color_picker::{ColorPicker, ColorPickerEvent, ColorPickerState};
use gpui_kit::component::group_box::GroupBoxVariant;
use gpui_kit::component::setting::{
    NumberFieldOptions, SettingField, SettingGroup, SettingItem, SettingPage, Settings,
};
use gpui_kit::component::{ActiveTheme as _, Icon, Sizable as _, TitleBar, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use jig_commands::{Preset, presets};

use crate::languages::{self, Language};
use crate::settings::{
    self, DEFAULT_FONT_SIZE, EditorSettings, MAX_FONT_SIZE, MIN_FONT_SIZE, ThemeChoice,
};
use crate::theme::{self, EDITABLE, Section};
use crate::workspace::{AddCommand, EditCommands, EditModelConfig};

actions!(jig, [OpenSettings]);

const PALETTE: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M12 22a1 1 0 0 1 0-20 10 9 0 0 1 10 9 5 5 0 0 1-5 5h-2.25a1.75 1.75 0 0 0-1.4 2.8l.3.4a1.75 1.75 0 0 1-1.4 2.8z"/><circle cx="13.5" cy="6.5" r=".5" fill="black"/><circle cx="17.5" cy="10.5" r=".5" fill="black"/><circle cx="6.5" cy="12.5" r=".5" fill="black"/><circle cx="8.5" cy="7.5" r=".5" fill="black"/></svg>"#;
const COMMAND: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M15 6v12a3 3 0 1 0 3-3H6a3 3 0 1 0 3 3V6a3 3 0 1 0-3 3h12a3 3 0 1 0-3-3"/></svg>"#;
const CODE: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="m16 18 6-6-6-6"/><path d="m8 6-6 6 6 6"/></svg>"#;
const SWATCHES: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M11 17a4 4 0 0 1-8 0V5a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2Z"/><path d="M16.7 13H19a2 2 0 0 1 2 2v4a2 2 0 0 1-2 2H7"/><path d="M7 17h.01"/><path d="m11 8 2.3-2.3a2.4 2.4 0 0 1 3.404.004L18.6 7.6a2.4 2.4 0 0 1 .026 3.434L9.9 19.8"/></svg>"#;
const SPARKLES: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M11.017 2.814a1 1 0 0 1 1.966 0l1.051 5.558a2 2 0 0 0 1.594 1.594l5.558 1.051a1 1 0 0 1 0 1.966l-5.558 1.051a2 2 0 0 0-1.594 1.594l-1.051 5.558a1 1 0 0 1-1.966 0l-1.051-5.558a2 2 0 0 0-1.594-1.594l-5.558-1.051a1 1 0 0 1 0-1.966l5.558-1.051a2 2 0 0 0 1.594-1.594z"/><path d="M20 2v4"/><path d="M22 4h-4"/><circle cx="4" cy="20" r="2"/></svg>"#;

/// Longest prompt shown under a command's name.
const MAX_PROMPT_CHARS: usize = 90;

/// The Settings window, while it's open.
struct OpenWindow(AnyWindowHandle);

impl Global for OpenWindow {}

pub fn init(cx: &mut App) {
    cx.bind_keys([KeyBinding::new("secondary-,", OpenSettings, None)]);
    cx.on_action(|_: &OpenSettings, cx| open(cx));
}

/// Show the Settings window, opening it if needed.
pub fn open(cx: &mut App) {
    if let Some(OpenWindow(handle)) = cx.try_global::<OpenWindow>()
        && handle
            .update(cx, |_, window, _| window.activate_window())
            .is_ok()
    {
        return;
    }
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::centered(size(px(760.), px(560.)), cx)),
        window_min_size: Some(size(px(560.), px(400.))),
        titlebar: Some(TitleBar::title_bar_options()),
        ..TitleBar::window_options()
    };
    match gpui_kit::open_window(options, cx, |window, cx| {
        cx.new(|cx| SettingsWindow::new(window, cx))
    }) {
        Ok((handle, _)) => cx.set_global(OpenWindow(handle)),
        Err(error) => eprintln!("jig: couldn't open Settings: {error:#}"),
    }
    cx.activate(true);
}

/// Whether `window` is the Settings window.
pub fn is_settings_window(window: AnyWindowHandle, cx: &App) -> bool {
    cx.try_global::<OpenWindow>()
        .is_some_and(|OpenWindow(handle)| *handle == window)
}

/// Run `action` in the frontmost editor window, bringing it forward.
fn send_to_workspace(action: Box<dyn Action>, cx: &mut App) {
    cx.defer(move |cx| {
        let windows = cx.window_stack().unwrap_or_else(|| cx.windows());
        let Some(target) = windows
            .into_iter()
            .find(|window| !is_settings_window(*window, cx))
        else {
            return;
        };
        target
            .update(cx, |_, window, cx| {
                window.activate_window();
                window.dispatch_action(action, cx);
            })
            .ok();
        cx.activate(true);
    });
}

/// A color as the theme shows it now: the user's, or Ember's own.
fn color_in_use(key: &str, cx: &App) -> Option<Hsla> {
    let dark = cx.theme().is_dark();
    settings::get(cx)
        .colors
        .variant(dark)
        .get(key)
        .and_then(|hex| theme::parse_hex(hex))
        .or_else(|| theme::default_color(key, dark))
}

/// Whether the user has changed a color in the variant in use.
fn is_changed(key: &str, cx: &App) -> bool {
    settings::get(cx)
        .colors
        .variant(cx.theme().is_dark())
        .contains_key(key)
}

/// Change a color in the variant in use; `None`, or Ember's own color,
/// goes back to Ember's.
fn set_color(key: &str, color: Option<Hsla>, cx: &mut App) {
    let dark = cx.theme().is_dark();
    let hex = color
        .map(theme::to_hex)
        .filter(|hex| theme::default_color(key, dark).map(theme::to_hex).as_ref() != Some(hex));
    settings::update(cx, |s| {
        let colors = s.colors.variant_mut(dark);
        match hex {
            Some(hex) => colors.insert(key.to_string(), hex),
            None => colors.remove(key),
        };
    });
}

/// A command as the Commands page lists it.
#[derive(Clone)]
struct CommandRow {
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
        Self {
            name: preset.name.clone().into(),
            detail: format!("{} · {prompt}", preset.scope.label()).into(),
        }
    }
}

/// What the Model page offers, read from `config.toml`.
#[derive(Clone)]
struct Providers {
    /// (name, label) for the dropdown.
    options: Vec<(SharedString, SharedString)>,
    /// The config file's own default.
    default: SharedString,
    /// Per provider: the key variable it needs and whether it's set.
    keys: Vec<(SharedString, Option<(String, bool)>)>,
}

pub struct SettingsWindow {
    /// The user's own commands, then the built-in ones they haven't replaced.
    user_commands: Vec<CommandRow>,
    built_in_commands: Vec<CommandRow>,
    commands_error: Option<SharedString>,
    providers: Result<Providers, SharedString>,
    /// One per entry of `theme::EDITABLE`, in the same order.
    color_pickers: Vec<Entity<ColorPickerState>>,
    _subscriptions: Vec<Subscription>,
}

impl SettingsWindow {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        crate::theme::sync(window, cx);
        let subscriptions = vec![
            cx.observe_window_appearance(window, |_, window, cx| crate::theme::sync(window, cx)),
            cx.observe_global::<settings::AppSettings>(|_, cx| cx.notify()),
            // Commands or providers may have been edited in the meantime.
            cx.observe_window_activation(window, |this, window, cx| {
                if window.is_window_active() {
                    this.reload();
                    cx.notify();
                }
            }),
        ];
        let mut subscriptions = subscriptions;
        let color_pickers = EDITABLE
            .iter()
            .map(|editable| {
                let picker = cx.new(|cx| ColorPickerState::new(window, cx));
                let key = editable.key;
                subscriptions.push(cx.subscribe_in(
                    &picker,
                    window,
                    move |_, _, event: &ColorPickerEvent, _, cx| {
                        if let ColorPickerEvent::Change(Some(color)) = event {
                            set_color(key, Some(*color), cx);
                        }
                    },
                ));
                picker
            })
            .collect();
        window.set_window_title("Settings");
        let mut this = Self {
            user_commands: Vec::new(),
            built_in_commands: Vec::new(),
            commands_error: None,
            providers: Err(SharedString::default()),
            color_pickers,
            _subscriptions: subscriptions,
        };
        this.reload();
        this
    }

    fn reload(&mut self) {
        self.reload_commands();
        self.reload_providers();
    }

    fn reload_commands(&mut self) {
        let path = presets::user_commands_path();
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
                    Some(format!("Your commands file has a problem: {error:#}").into());
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

    fn reload_providers(&mut self) {
        self.providers = jig_ai::Config::load(jig_ai::Config::user_path().as_deref())
            .map(|config| Providers {
                options: config
                    .providers
                    .iter()
                    .map(|p| {
                        (
                            p.name.clone().into(),
                            format!("{} · {}", p.name, p.model).into(),
                        )
                    })
                    .collect(),
                default: config.default.clone().into(),
                keys: config
                    .providers
                    .iter()
                    .map(|p| {
                        let key = p.api_key_env.clone().map(|var| (var, p.has_key()));
                        (p.name.clone().into(), key)
                    })
                    .collect(),
            })
            .map_err(|error| format!("{error:#}").into());
    }

    fn appearance_page(&self) -> SettingPage {
        let theme_options = ThemeChoice::ALL
            .into_iter()
            .map(|choice| (choice.key().into(), choice.label().into()))
            .collect();
        let editor_switch = |title: &'static str,
                             description: &'static str,
                             get: fn(&EditorSettings) -> bool,
                             set: fn(&mut EditorSettings, bool)| {
            SettingItem::new(
                title,
                SettingField::switch(
                    move |cx| get(&settings::get(cx).editor),
                    move |value, cx| settings::update(cx, |s| set(&mut s.editor, value)),
                )
                .default_value(get(&EditorSettings::default())),
            )
            .description(description)
        };

        SettingPage::new("Appearance")
            .icon(Icon::default().data(PALETTE))
            .group(
                SettingGroup::new()
                    .title("Theme")
                    .item(
                        SettingItem::new(
                            "Appearance",
                            SettingField::dropdown(
                                theme_options,
                                |cx| settings::get(cx).appearance.theme.key().into(),
                                |key: SharedString, cx| {
                                    settings::update(cx, |s| {
                                        s.appearance.theme = ThemeChoice::from_key(&key)
                                    })
                                },
                            )
                            .default_value(ThemeChoice::System.key()),
                        )
                        .description("Ember Light or Ember, or follow the system."),
                    )
                    .item(
                        SettingItem::new(
                            "Code font size",
                            SettingField::number_input(
                                NumberFieldOptions {
                                    min: MIN_FONT_SIZE.into(),
                                    max: MAX_FONT_SIZE.into(),
                                    step: 0.5,
                                },
                                |cx| settings::get(cx).appearance.font_size.into(),
                                |size, cx| {
                                    let size = (size as f32).clamp(MIN_FONT_SIZE, MAX_FONT_SIZE);
                                    settings::update(cx, |s| s.appearance.font_size = size)
                                },
                            )
                            .default_value(f64::from(DEFAULT_FONT_SIZE)),
                        )
                        .description("In points."),
                    ),
            )
            .group(
                SettingGroup::new()
                    .title("Editor")
                    .item(editor_switch(
                        "Line numbers",
                        "Show line numbers beside the code.",
                        |e| e.line_numbers,
                        |e, v| e.line_numbers = v,
                    ))
                    .item(editor_switch(
                        "Wrap long lines",
                        "Wrap lines at the window's edge instead of scrolling sideways.",
                        |e| e.soft_wrap,
                        |e, v| e.soft_wrap = v,
                    ))
                    .item(editor_switch(
                        "Indent guides",
                        "Draw a faint line at each indentation level.",
                        |e| e.indent_guides,
                        |e, v| e.indent_guides = v,
                    ))
                    .item(editor_switch(
                        "Show whitespace",
                        "Mark spaces and tabs.",
                        |e| e.show_whitespace,
                        |e, v| e.show_whitespace = v,
                    )),
            )
    }

    /// Show each picker the color in use, unless it's being picked.
    fn sync_color_pickers(&self, window: &mut Window, cx: &mut App) {
        for (editable, picker) in EDITABLE.iter().zip(&self.color_pickers) {
            let Some(color) = color_in_use(editable.key, cx) else {
                continue;
            };
            let state = picker.read(cx);
            if state.is_open() || state.value().map(theme::to_hex) == Some(theme::to_hex(color)) {
                continue;
            }
            picker.update(cx, |state, cx| state.set_value(color, window, cx));
        }
    }

    fn colors_page(&self, cx: &App) -> SettingPage {
        let variant = if cx.theme().is_dark() {
            "Ember"
        } else {
            "Ember Light"
        };
        let color_item = |(editable, picker): (&theme::Editable, &Entity<ColorPickerState>)| {
            let key = editable.key;
            let picker = picker.clone();
            SettingItem::new(
                editable.label,
                SettingField::render(move |_, _, cx| {
                    let hex = color_in_use(key, cx).map(theme::to_hex).unwrap_or_default();
                    h_flex()
                        .gap_2()
                        .items_center()
                        .child(
                            div()
                                .text_xs()
                                .font_family(cx.theme().mono_font_family.clone())
                                .text_color(cx.theme().muted_foreground)
                                .child(hex),
                        )
                        .child(ColorPicker::new(&picker).small().anchor(Anchor::TopRight))
                })
                .on_reset(
                    move |cx| is_changed(key, cx),
                    move |_, cx| set_color(key, None, cx),
                ),
            )
            .keywords([key])
        };
        let section = |section: Section| {
            EDITABLE
                .iter()
                .zip(&self.color_pickers)
                .filter(move |(editable, _)| editable.section == section)
                .map(color_item)
        };

        let intro = SettingItem::render(move |_, _, cx| {
            let any_changed = EDITABLE.iter().any(|editable| is_changed(editable.key, cx));
            v_flex()
                .gap_2()
                .child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child(format!(
                            "Changes apply to {variant}, the variant in use. Switch \
                             Appearance to edit the other."
                        )),
                )
                .when(any_changed, |this| {
                    this.child(
                        h_flex().child(
                            Button::new("reset-colors")
                                .label(format!("Reset All of {variant}"))
                                .small()
                                .outline()
                                .on_click(|_, _, cx| {
                                    let dark = cx.theme().is_dark();
                                    settings::update(cx, |s| s.colors.variant_mut(dark).clear())
                                }),
                        ),
                    )
                })
        })
        .keywords(["theme", "color", "scheme", "reset"]);

        SettingPage::new("Colors")
            .icon(Icon::default().data(SWATCHES))
            .group(
                SettingGroup::new()
                    .variant(GroupBoxVariant::Normal)
                    .item(intro),
            )
            .group(
                SettingGroup::new()
                    .title("Code")
                    .items(section(Section::Code)),
            )
            .group(
                SettingGroup::new()
                    .title("Editor")
                    .items(section(Section::Editor)),
            )
            .group(
                SettingGroup::new()
                    .title("Interface")
                    .items(section(Section::Interface)),
            )
    }

    fn languages_page(&self) -> SettingPage {
        let highlight_item = |language: &'static Language| {
            SettingItem::new(
                language.label,
                SettingField::switch(
                    |cx| !settings::get(cx).languages.is_off(language.name),
                    |on, cx| settings::update(cx, |s| s.languages.set_off(language.name, !on)),
                )
                .default_value(true),
            )
        };
        let extensions_item = |language: &'static Language| {
            let built_in = languages::format_extensions(language.extensions);
            let shown = built_in.clone();
            SettingItem::new(
                language.label,
                SettingField::input(
                    move |cx| {
                        settings::get(cx)
                            .languages
                            .extensions
                            .get(language.name)
                            .cloned()
                            .unwrap_or_else(|| shown.clone())
                            .into()
                    },
                    move |text: SharedString, cx| {
                        let text = Some(text.to_string());
                        settings::update(cx, |s| {
                            s.languages.set_extensions(language.name, text, &built_in)
                        })
                    },
                )
                .default_value(languages::format_extensions(language.extensions)),
            )
            .keywords([language.name])
        };

        SettingPage::new("Languages")
            .icon(Icon::default().data(CODE))
            .group(
                SettingGroup::new()
                    .title("Highlighting")
                    .description("Languages turned off open as plain text.")
                    .items(languages::BUNDLED.iter().map(highlight_item)),
            )
            .group(
                SettingGroup::new()
                    .title("File extensions")
                    .description(
                        "Which files each language applies to, separated by commas. \
                         Where two languages share one, the higher wins.",
                    )
                    .items(languages::BUNDLED.iter().map(extensions_item)),
            )
    }

    fn commands_page(&self) -> SettingPage {
        let command_item = |row: &CommandRow| {
            let name = row.name.clone();
            let toggled = row.name.clone();
            SettingItem::new(
                row.name.clone(),
                SettingField::switch(
                    move |cx| !settings::get(cx).commands.is_hidden(&name),
                    move |shown, cx| {
                        settings::update(cx, |s| s.commands.set_hidden(&toggled, !shown))
                    },
                )
                .default_value(true),
            )
            .description(row.detail.clone())
        };

        let actions = SettingItem::render(|_, _, cx| {
            v_flex()
                .gap_2()
                .child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child(
                            "Commands you turn off leave the command input (⌘K) but stay \
                             defined, so you can turn them back on here.",
                        ),
                )
                .child(
                    h_flex()
                        .gap_2()
                        .child(
                            Button::new("add-command")
                                .label("Add Command…")
                                .small()
                                .primary()
                                .on_click(|_, _, cx| send_to_workspace(Box::new(AddCommand), cx)),
                        )
                        .child(
                            Button::new("edit-commands")
                                .label("Edit Commands File")
                                .small()
                                .outline()
                                .on_click(|_, _, cx| send_to_workspace(Box::new(EditCommands), cx)),
                        ),
                )
        })
        .keywords(["add", "new", "file", "toml"]);

        let mut page = SettingPage::new("Commands")
            .icon(Icon::default().data(COMMAND))
            .group(
                SettingGroup::new()
                    .variant(GroupBoxVariant::Normal)
                    .item(actions),
            );
        if let Some(error) = self.commands_error.clone() {
            page = page.group(
                SettingGroup::new().item(SettingItem::render(move |_, _, cx| {
                    div()
                        .text_sm()
                        .text_color(cx.theme().danger)
                        .child(error.clone())
                })),
            );
        }
        if !self.user_commands.is_empty() {
            page = page.group(
                SettingGroup::new()
                    .title("Your commands")
                    .description("From ~/.config/jig/commands.toml.")
                    .items(self.user_commands.iter().map(command_item)),
            );
        }
        page.group(
            SettingGroup::new()
                .title("Built-in")
                .items(self.built_in_commands.iter().map(command_item)),
        )
    }

    fn model_page(&self) -> SettingPage {
        let edit_file = SettingItem::render(|_, _, cx| {
            v_flex()
                .gap_2()
                .child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child(
                            "Providers live in ~/.config/jig/config.toml. API keys are \
                             read from environment variables, never stored by Jig.",
                        ),
                )
                .child(
                    h_flex().child(
                        Button::new("edit-config")
                            .label("Edit Providers File")
                            .small()
                            .outline()
                            .on_click(|_, _, cx| send_to_workspace(Box::new(EditModelConfig), cx)),
                    ),
                )
        })
        .keywords(["config", "provider", "api", "key"]);

        let page = SettingPage::new("Model").icon(Icon::default().data(SPARKLES));
        let providers = match &self.providers {
            Ok(providers) => providers.clone(),
            Err(error) => {
                let error = error.clone();
                return page.group(
                    SettingGroup::new()
                        .item(SettingItem::render(move |_, _, cx| {
                            div()
                                .text_sm()
                                .text_color(cx.theme().danger)
                                .child(format!("Couldn't read your providers. {error}"))
                        }))
                        .item(edit_file),
                );
            }
        };

        let chosen = {
            let providers = providers.clone();
            move |cx: &App| -> SharedString {
                settings::get(cx)
                    .ai
                    .provider
                    .map(SharedString::from)
                    .filter(|name| providers.options.iter().any(|(option, _)| option == name))
                    .unwrap_or_else(|| providers.default.clone())
            }
        };
        let default = providers.default.clone();
        let key_status = {
            let chosen = chosen.clone();
            let providers = providers.clone();
            SettingField::render(move |_, _, cx| {
                let name = chosen(cx);
                let key = providers
                    .keys
                    .iter()
                    .find(|(provider, _)| *provider == name)
                    .and_then(|(_, key)| key.clone());
                let (text, ok) = match key {
                    None => ("Not needed".to_string(), true),
                    Some((var, true)) => (format!("{var} is set"), true),
                    Some((var, false)) => (format!("{var} is not set"), false),
                };
                div()
                    .text_sm()
                    .text_color(if ok {
                        cx.theme().muted_foreground
                    } else {
                        cx.theme().danger
                    })
                    .child(text)
            })
        };

        page.group(
            SettingGroup::new()
                .title("Provider")
                .item(
                    SettingItem::new(
                        "Model",
                        SettingField::dropdown(
                            providers.options.clone(),
                            chosen,
                            move |name: SharedString, cx| {
                                // Picking the file's default clears the choice,
                                // so a later change of default applies.
                                let choice = (name != default).then(|| name.to_string());
                                settings::update(cx, |s| s.ai.provider = choice)
                            },
                        )
                        .default_value(providers.default.clone()),
                    )
                    .description("The model every command uses."),
                )
                .item(
                    SettingItem::new("API key", key_status)
                        .description("Set in the environment Jig was started from."),
                ),
        )
        .group(
            SettingGroup::new()
                .variant(GroupBoxVariant::Normal)
                .item(edit_file),
        )
    }
}

impl Render for SettingsWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.sync_color_pickers(window, cx);
        let colors_page = self.colors_page(cx);
        let theme = cx.theme();
        v_flex()
            .size_full()
            .bg(theme.background)
            .text_color(theme.foreground)
            .child(
                TitleBar::new().child(
                    h_flex()
                        .flex_1()
                        .justify_center()
                        .text_sm()
                        .font_weight(FontWeight::SEMIBOLD)
                        // Balances the traffic lights so the title is centred.
                        .pr(px(72.))
                        .child("Settings"),
                ),
            )
            .child(
                div().flex_1().min_h_0().child(
                    Settings::new("jig-settings")
                        .sidebar_width(px(200.))
                        .sidebar_size_range(px(170.)..px(280.))
                        .with_group_variant(GroupBoxVariant::Outline)
                        .pages([
                            self.appearance_page(),
                            colors_page,
                            self.languages_page(),
                            self.commands_page(),
                            self.model_page(),
                        ]),
                ),
            )
    }
}

#[cfg(test)]
mod tests {
    use gpui_kit::component::ActiveTheme as _;
    use gpui_kit::{AppContext as _, TestAppContext};
    use jig_commands::Preset;

    use super::{CommandRow, MAX_PROMPT_CHARS, init, is_settings_window, open, set_color};
    use crate::settings::{self, ThemeChoice};

    #[test]
    fn long_prompts_are_shortened_to_one_line() {
        let row = CommandRow::new(&Preset {
            name: "A".into(),
            prompt: format!("first line\n  second {}", "word ".repeat(40)),
            ..Default::default()
        });
        assert!(row.detail.starts_with("selection · first line second word"));
        assert!(row.detail.ends_with('…'));
        assert!(row.detail.chars().count() < MAX_PROMPT_CHARS + 20);
    }

    #[gpui_kit::test]
    fn opens_once_and_reflects_changes(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            crate::theme::init(cx);
            init(cx);
            open(cx);
            open(cx);
        });
        cx.run_until_parked();
        let windows = cx.update(|cx| cx.windows());
        assert_eq!(windows.len(), 1, "a second ⌘, reuses the window");
        assert!(cx.update(|cx| is_settings_window(windows[0], cx)));

        cx.update(|cx| {
            settings::update(cx, |s| s.appearance.theme = ThemeChoice::Dark);
        });
        cx.run_until_parked();
        cx.update_window(windows[0], |_, window, cx| window.draw(cx).clear(cx))
            .unwrap();
        assert!(cx.update(|cx| cx.theme().is_dark()));
    }

    #[gpui_kit::test]
    fn picking_ember_s_own_color_clears_the_change(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            crate::theme::init(cx);
            settings::update(cx, |s| s.appearance.theme = ThemeChoice::Dark);
        });
        cx.update(|cx| {
            set_color("syntax.keyword", crate::theme::parse_hex("#FF0000"), cx);
            assert_eq!(settings::get(cx).colors.dark["syntax.keyword"], "#FF0000");
            assert!(
                settings::get(cx).colors.light.is_empty(),
                "only the variant in use"
            );

            set_color(
                "syntax.keyword",
                crate::theme::default_color("syntax.keyword", true),
                cx,
            );
            assert!(settings::get(cx).colors.dark.is_empty());
        });
    }
}
