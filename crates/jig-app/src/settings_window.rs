//! The Settings window (⌘,): Appearance, Colors, Languages, Shortcuts,
//! Jigs, Model and Agent.

mod agent_page;
mod shortcuts_page;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::color_picker::{ColorPicker, ColorPickerEvent, ColorPickerState};
use gpui_kit::component::group_box::GroupBoxVariant;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::setting::{
    NumberFieldOptions, SettingField, SettingGroup, SettingItem, SettingPage, Settings,
};
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, Sizable as _, TitleBar, h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use jig_ai::{Config, ProviderConfig, ProviderTemplate, TEMPLATES, template_named};
use jig_commands::{Preset, presets};

use crate::debuggers::{self, Debugger};
use crate::formatters;
use crate::languages::{self, Language};
use crate::providers::{self, ModelList};
use crate::settings::{
    self, DEFAULT_FONT_SIZE, DEFAULT_TRANSLUCENCY, EditorSettings, MAX_FONT_SIZE, MAX_TRANSLUCENCY,
    MIN_FONT_SIZE, ThemeChoice,
};
use crate::theme::{self, EDITABLE, Section};
use crate::workspace::{AddCommand, EditCommands, EditDebuggers, EditFormatters, EditModelConfig};

actions!(jig, [OpenSettings]);

const PALETTE: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M12 22a1 1 0 0 1 0-20 10 9 0 0 1 10 9 5 5 0 0 1-5 5h-2.25a1.75 1.75 0 0 0-1.4 2.8l.3.4a1.75 1.75 0 0 1-1.4 2.8z"/><circle cx="13.5" cy="6.5" r=".5" fill="black"/><circle cx="17.5" cy="10.5" r=".5" fill="black"/><circle cx="6.5" cy="12.5" r=".5" fill="black"/><circle cx="8.5" cy="7.5" r=".5" fill="black"/></svg>"#;
const COMMAND: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M15 6v12a3 3 0 1 0 3-3H6a3 3 0 1 0 3 3V6a3 3 0 1 0-3 3h12a3 3 0 1 0-3-3"/></svg>"#;
const CODE: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="m16 18 6-6-6-6"/><path d="m8 6-6 6 6 6"/></svg>"#;
const SWATCHES: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M11 17a4 4 0 0 1-8 0V5a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2Z"/><path d="M16.7 13H19a2 2 0 0 1 2 2v4a2 2 0 0 1-2 2H7"/><path d="M7 17h.01"/><path d="m11 8 2.3-2.3a2.4 2.4 0 0 1 3.404.004L18.6 7.6a2.4 2.4 0 0 1 .026 3.434L9.9 19.8"/></svg>"#;
const SPARKLES: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M11.017 2.814a1 1 0 0 1 1.966 0l1.051 5.558a2 2 0 0 0 1.594 1.594l5.558 1.051a1 1 0 0 1 0 1.966l-5.558 1.051a2 2 0 0 0-1.594 1.594l-1.051 5.558a1 1 0 0 1-1.966 0l-1.051-5.558a2 2 0 0 0-1.594-1.594l-5.558-1.051a1 1 0 0 1 0-1.966l5.558-1.051a2 2 0 0 0 1.594-1.594z"/><path d="M20 2v4"/><path d="M22 4h-4"/><circle cx="4" cy="20" r="2"/></svg>"#;

/// Longest prompt shown under a command's name.
const MAX_PROMPT_CHARS: usize = 90;

#[cfg(test)]
thread_local! {
    /// Which page a test's Settings window opens on.
    static START_PAGE: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

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

/// Work Settings started for a provider: loading its models, or testing it.
enum Fetch<T> {
    Running,
    Done(T),
    Failed(SharedString),
}

pub struct SettingsWindow {
    /// The user's own commands, then the built-in ones they haven't replaced.
    user_commands: Vec<CommandRow>,
    built_in_commands: Vec<CommandRow>,
    commands_error: Option<SharedString>,
    /// A masked key field per provider that takes a key, in `TEMPLATES`
    /// order. A key is saved as it's typed and never read back in.
    key_inputs: Vec<(&'static ProviderTemplate, Entity<InputState>)>,
    /// From Install, per debugger key.
    installs: HashMap<String, Fetch<()>>,
    /// From Test: how long the quick model took to answer.
    check: Option<Fetch<SharedString>>,
    /// Why the last change to the providers didn't work.
    providers_error: Option<SharedString>,
    /// One per entry of `theme::EDITABLE`, in the same order.
    color_pickers: Vec<Entity<ColorPickerState>>,
    /// The shortcut taking new keys, on the Shortcuts page.
    recording: Option<shortcuts_page::Recording>,
    _subscriptions: Vec<Subscription>,
}

impl SettingsWindow {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        crate::theme::sync(window, cx);
        let subscriptions = vec![
            cx.observe_window_appearance(window, |_, window, cx| crate::theme::sync(window, cx)),
            cx.observe_global::<settings::AppSettings>(|_, cx| cx.notify()),
            cx.observe_global::<providers::Providers>(|_, cx| cx.notify()),
            // Commands or providers may have been edited in the meantime.
            cx.observe_window_activation(window, |this, window, cx| {
                if window.is_window_active() {
                    this.reload_commands();
                    providers::reload(cx);
                    providers::load_all_models(true, cx);
                    cx.notify();
                }
            }),
        ];
        let mut subscriptions = subscriptions;
        let key_inputs = TEMPLATES
            .iter()
            .filter(|template| template.api_key_env.is_some())
            .map(|template| {
                let input = cx.new(|cx| {
                    InputState::new(window, cx)
                        .masked(true)
                        .placeholder("Paste an API key")
                });
                subscriptions.push(cx.subscribe(&input, move |this, input, event, cx| {
                    if let InputEvent::Change = event {
                        let key = input.read(cx).value();
                        if !key.trim().is_empty() {
                            let result = providers::connect(template, Some(&key), cx);
                            this.report(result, cx);
                        }
                    }
                }));
                (template, input)
            })
            .collect();
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
            key_inputs,
            installs: HashMap::new(),
            check: None,
            providers_error: None,
            color_pickers,
            recording: None,
            _subscriptions: subscriptions,
        };
        this.reload_commands();
        providers::load_all_models(true, cx);
        this
    }

    fn reload_commands(&mut self) {
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

        let mut theme_group = SettingGroup::new()
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
            );
        // Only macOS blurs what is behind the window.
        if cfg!(target_os = "macos") {
            theme_group = theme_group.item(
                SettingItem::new(
                    "Translucency",
                    SettingField::number_input(
                        NumberFieldOptions {
                            min: 0.,
                            max: MAX_TRANSLUCENCY.into(),
                            step: 5.,
                        },
                        |cx| settings::get(cx).appearance.translucency.into(),
                        |percent, cx| {
                            let percent = (percent as f32).clamp(0., MAX_TRANSLUCENCY);
                            settings::update(cx, |s| s.appearance.translucency = percent)
                        },
                    )
                    .default_value(f64::from(DEFAULT_TRANSLUCENCY)),
                )
                .description(
                    "How much of the blurred desktop shows through the editor, in percent.",
                ),
            );
        }

        SettingPage::new("Appearance")
            .icon(Icon::default().data(PALETTE))
            .group(theme_group)
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
                    ))
                    .item(editor_switch(
                        "Autocomplete",
                        "Suggest names while typing, from the language server or words in the file.",
                        |e| e.autocomplete,
                        |e, v| e.autocomplete = v,
                    ))
                    .item(editor_switch(
                        "Jig button on selections",
                        "Show a button beside selected code that opens the ⌘K input.",
                        |e| e.selection_button,
                        |e, v| e.selection_button = v,
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

    fn languages_page(&self, cx: &Context<Self>) -> SettingPage {
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

        let registry = debuggers::registry();
        let debug_item = |debugger: &Arc<Debugger>| {
            let (status, _) = debuggers::status(debugger);
            let (get, set) = (debugger.key.clone(), debugger.key.clone());
            SettingItem::new(
                debugger.name.clone(),
                SettingField::switch(
                    move |cx| settings::get(cx).debugging.is_enabled(&get),
                    move |on, cx| settings::update(cx, |s| s.debugging.set_enabled(&set, on)),
                )
                .default_value(debugger.key == "rust"),
            )
            .description(status)
            .keywords(["debug", "debugger", "breakpoint"])
        };
        let debug_items = |debugger: &Arc<Debugger>| {
            let mut items = vec![debug_item(debugger)];
            if debugger.installable() {
                items.push(self.install_item(debugger.clone(), cx));
            }
            items
        };
        let edit_item = SettingItem::render({
            let error = registry.error.clone();
            move |_, _, cx| {
                v_flex()
                    .gap_2()
                    .child(
                        div()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child(
                                "Any debugger that speaks the Debug Adapter Protocol can be \
                                 added, or a built-in one changed, in debuggers.toml.",
                            ),
                    )
                    .when_some(error.clone(), |this, error| {
                        this.child(div().text_sm().text_color(cx.theme().danger).child(error))
                    })
                    .child(
                        h_flex().child(
                            Button::new("edit-debuggers")
                                .label("Edit Debuggers File")
                                .small()
                                .outline()
                                .on_click(|_, _, cx| {
                                    send_to_workspace(Box::new(EditDebuggers), cx)
                                }),
                        ),
                    )
            }
        })
        .keywords(["debug", "debugger", "add", "file", "toml"]);

        let formatting = formatters::registry();
        let format_item = |language: &'static Language| {
            // What formats it when its language server doesn't.
            let fallback = match formatting.for_language(language.name, std::path::Path::new("/")) {
                Some(formatter) => format!("Its language server, or {}.", formatter.name),
                None => match formatting.knows(language.name) {
                    Some(formatter) => format!(
                        "Its language server, or {} once it's installed.",
                        formatter.name
                    ),
                    None => "Its language server, if it formats.".into(),
                },
            };
            SettingItem::new(
                language.label,
                SettingField::switch(
                    |cx| settings::get(cx).formatting.on_save(language.name),
                    |on, cx| settings::update(cx, |s| s.formatting.set_on_save(language.name, on)),
                )
                .default_value(false),
            )
            .description(fallback)
            .keywords(["format", "formatter", "save", language.name])
        };
        let tidy_item = |label: &'static str,
                         description: &'static str,
                         get: fn(&settings::FormatSettings) -> bool,
                         set: fn(&mut settings::FormatSettings, bool)| {
            SettingItem::new(
                label,
                SettingField::switch(
                    move |cx| get(&settings::get(cx).formatting),
                    move |on, cx| settings::update(cx, |s| set(&mut s.formatting, on)),
                )
                .default_value(false),
            )
            .description(description)
            .keywords(["format", "save", "whitespace", "newline"])
        };
        let formatters_item = SettingItem::render({
            let error = formatting.error.clone();
            move |_, _, cx| {
                v_flex()
                    .gap_2()
                    .child(
                        div()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child(
                                "Formatters for files whose language server doesn't format are \
                         added, or a built-in one changed, in formatters.toml.",
                            ),
                    )
                    .when_some(error.clone(), |this, error| {
                        this.child(div().text_sm().text_color(cx.theme().danger).child(error))
                    })
                    .child(
                        h_flex().child(
                            Button::new("edit-formatters")
                                .label("Edit Formatters File")
                                .small()
                                .outline()
                                .on_click(|_, _, cx| {
                                    send_to_workspace(Box::new(EditFormatters), cx)
                                }),
                        ),
                    )
            }
        })
        .keywords(["format", "formatter", "add", "file", "toml"]);

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
            .group(
                SettingGroup::new()
                    .title("Debugging")
                    .description(
                        "Languages turned on take breakpoints and debug with ⌃D. Each \
                         says where Jig found its debugger, or how to get it.",
                    )
                    .items(registry.debuggers.iter().flat_map(debug_items))
                    .item(edit_item),
            )
            .group(
                SettingGroup::new()
                    .title("On save")
                    .item(tidy_item(
                        "Trim trailing whitespace",
                        "Spaces and tabs at the ends of lines are removed.",
                        |f| f.trim_trailing_whitespace,
                        |f, on| f.trim_trailing_whitespace = on,
                    ))
                    .item(tidy_item(
                        "End with a newline",
                        "A file that doesn't end with a line break gets one.",
                        |f| f.final_newline,
                        |f, on| f.final_newline = on,
                    )),
            )
            .group(
                SettingGroup::new()
                    .title("Format on save")
                    .description(
                        "Languages turned on are formatted as they're saved, as with ⇧⌥F. \
                         A formatter that takes over 2 seconds is skipped and the file \
                         saved as it is.",
                    )
                    .items(languages::BUNDLED.iter().map(format_item))
                    .item(formatters_item),
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
                            "Jigs you turn off leave the ⌘K input but stay defined, so you \
                             can turn them back on here.",
                        ),
                )
                .child(
                    h_flex()
                        .gap_2()
                        .child(
                            Button::new("add-command")
                                .label("Add Jig…")
                                .small()
                                .primary()
                                .on_click(|_, _, cx| send_to_workspace(Box::new(AddCommand), cx)),
                        )
                        .child(
                            Button::new("edit-commands")
                                .label("Edit Jigs File")
                                .small()
                                .outline()
                                .on_click(|_, _, cx| send_to_workspace(Box::new(EditCommands), cx)),
                        ),
                )
        })
        .keywords(["add", "new", "file", "toml"]);

        let mut page = SettingPage::new("Jigs")
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
                    .title("Your jigs")
                    .description("From ~/.config/jig/jigs.toml.")
                    .items(self.user_commands.iter().map(command_item)),
            );
        }
        page.group(
            SettingGroup::new()
                .title("Built-in")
                .items(self.built_in_commands.iter().map(command_item)),
        )
    }

    /// Show a failed change to the providers until the next one works.
    fn report(&mut self, result: Result<(), String>, cx: &mut Context<Self>) {
        self.providers_error = result.err().map(SharedString::from);
        cx.notify();
    }

    /// Run one small command on the quick model.
    fn check(&mut self, cx: &mut Context<Self>) {
        let provider = match providers::quick(cx) {
            Ok(provider) => provider,
            Err(error) => {
                self.check = Some(Fetch::Failed(error.into()));
                cx.notify();
                return;
            }
        };
        let key = providers::api_key(&provider, cx).map(|(key, _)| key);
        self.check = Some(Fetch::Running);
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let started = Instant::now();
                    jig_ai::check(provider.build_with(key)?.as_ref())?;
                    anyhow::Ok(started.elapsed())
                })
                .await;
            this.update(cx, |this, cx| {
                this.check = Some(match result {
                    Ok(took) => Fetch::Done(format!("Works · {:.1} s", took.as_secs_f32()).into()),
                    Err(error) => Fetch::Failed(format!("{error:#}").into()),
                });
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    fn model_page(&self, cx: &Context<Self>) -> SettingPage {
        let page = SettingPage::new("Model").icon(Icon::default().data(SPARKLES));
        let config = match providers::config(cx) {
            Ok(config) => config,
            Err(error) => {
                return page.group(
                    SettingGroup::new()
                        .item(SettingItem::render(move |_, _, cx| {
                            div()
                                .text_sm()
                                .text_color(cx.theme().danger)
                                .child(format!("Couldn't read your providers. {error}"))
                        }))
                        .item(edit_providers_file()),
                );
            }
        };
        let mut page = page;
        if let Some(error) = self.providers_error.clone() {
            page = page.group(
                SettingGroup::new().item(SettingItem::render(move |_, _, cx| {
                    div()
                        .text_sm()
                        .text_color(cx.theme().danger)
                        .child(error.clone())
                })),
            );
        }
        page = page
            .group(self.models_group(&config, cx))
            .group(self.providers_group(&config, cx));
        let mut others = SettingGroup::new().title("Other providers");
        let hand_added: Vec<&ProviderConfig> = config
            .providers
            .iter()
            .filter(|provider| template_named(&provider.name).is_none())
            .collect();
        if !hand_added.is_empty() {
            others = others.description("Added to config.toml by hand.");
        }
        for provider in hand_added {
            others = others.item(other_provider_item(provider, cx));
        }
        page.group(others.item(edit_providers_file()))
    }

    /// The model each lane uses, from what the connected providers offer.
    fn models_group(&self, config: &Config, cx: &Context<Self>) -> SettingGroup {
        let this = cx.entity().downgrade();
        let offered = offered_models(config, cx);
        let mut quick_options = offered.clone();
        if let Some(id) = config.quick_model() {
            keep_current(&mut quick_options, &id, config);
        }

        let mut agent_options = vec![(SharedString::from(""), "Same as Jig".into())];
        let agent = match (&config.agent, &config.agent_model) {
            (Some(id), _) => id.clone(),
            (None, Some(native)) => {
                let id = format!("{OPENCODE_OWN}{native}");
                agent_options.push((
                    id.clone().into(),
                    format!("{native} (from OpenCode)").into(),
                ));
                id
            }
            (None, None) => String::new(),
        };
        agent_options.extend(offered);
        if config.agent.is_some() {
            keep_current(&mut agent_options, &agent, config);
        }

        let quick_item = if quick_options.is_empty() {
            SettingItem::render(|_, _, cx| {
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child("Connect a provider below, then pick its models here.")
            })
        } else {
            let this = this.clone();
            SettingItem::new(
                "Jig",
                SettingField::scrollable_dropdown(
                    quick_options,
                    |cx| {
                        providers::config(cx)
                            .ok()
                            .and_then(|config| config.quick_model())
                            .unwrap_or_default()
                            .into()
                    },
                    move |id: SharedString, cx| {
                        let result = providers::set_quick(&id, cx);
                        this.update(cx, |this, cx| {
                            this.check = None;
                            this.report(result, cx);
                        })
                        .ok();
                    },
                ),
            )
            .description("One fast call at the cursor. Small models answer in 2-5 s.")
        };
        let agent_item = SettingItem::new(
            "Agent",
            SettingField::scrollable_dropdown(
                agent_options,
                move |_| agent.clone().into(),
                move |id: SharedString, cx| {
                    if id.starts_with(OPENCODE_OWN) {
                        return;
                    }
                    let id = (!id.is_empty()).then_some(id.as_ref());
                    let result = providers::set_agent(id, cx);
                    this.update(cx, |this, cx| this.report(result, cx)).ok();
                },
            ),
        )
        .description("Works across the project through OpenCode. A stronger model helps.");

        SettingGroup::new()
            .title("Models")
            .item(quick_item)
            .item(agent_item)
            .item(self.check_item(config, cx))
    }

    /// Test the quick model, and refresh every provider's models.
    fn check_item(&self, config: &Config, cx: &Context<Self>) -> SettingItem {
        let this = cx.entity().downgrade();
        let can_test = config.quick_model().is_some();
        let check = self.check.as_ref().map(|fetch| match fetch {
            Fetch::Running => (SharedString::from("Testing…"), true),
            Fetch::Done(result) => (result.clone(), true),
            Fetch::Failed(error) => (error.clone(), false),
        });
        let testing = matches!(self.check, Some(Fetch::Running));
        SettingItem::render(move |_, _, cx| {
            let test = this.clone();
            v_flex()
                .gap_1()
                .child(
                    h_flex()
                        .gap_2()
                        .child(
                            Button::new("test-quick")
                                .label("Test Jig Model")
                                .small()
                                .outline()
                                .disabled(testing || !can_test)
                                .on_click(move |_, _, cx| {
                                    test.update(cx, |this, cx| this.check(cx)).ok();
                                }),
                        )
                        .child(
                            Button::new("refresh-models")
                                .label("Refresh Models")
                                .small()
                                .ghost()
                                .on_click(|_, _, cx| providers::load_all_models(false, cx)),
                        ),
                )
                .when_some(check.clone(), |this, (text, ok)| {
                    this.child(status_text(text, ok, cx))
                })
        })
        .keywords(["test", "check", "refresh", "models"])
    }

    /// Every major provider, with a key field or a switch to connect it.
    fn providers_group(&self, config: &Config, cx: &Context<Self>) -> SettingGroup {
        let mut group = SettingGroup::new().title("Providers").description(
            "Paste a key to connect. Keys go in the system keychain, never in config.toml.",
        );
        for template in TEMPLATES {
            let provider = config.provider(template.name).cloned();
            let item = match self
                .key_inputs
                .iter()
                .find(|(keyed, _)| keyed.name == template.name)
            {
                Some((_, input)) => keyed_item(template, provider, input.clone(), cx),
                None => local_item(template, provider, cx),
            };
            group = group.item(item);
        }
        group
    }

    /// Install (or reinstall) a debugger, and how that went.
    fn install_item(&self, debugger: Arc<Debugger>, cx: &Context<Self>) -> SettingItem {
        let this = cx.entity().downgrade();
        let fetch = self.installs.get(&debugger.key);
        let installing = matches!(fetch, Some(Fetch::Running));
        let (_, ready) = debuggers::status(&debugger);
        let report = fetch.map(|fetch| match fetch {
            Fetch::Running => (SharedString::from("Installing…"), true),
            Fetch::Done(()) => ("Installed.".into(), true),
            Fetch::Failed(error) => (error.clone(), false),
        });
        let label = if ready { "Reinstall" } else { "Install" };
        let id = SharedString::from(format!("install-{}", debugger.key));
        SettingItem::render(move |_, _, cx| {
            let this = this.clone();
            let debugger = debugger.clone();
            v_flex()
                .gap_1()
                .child(
                    h_flex().gap_2().items_center().child(
                        Button::new(id.clone())
                            .label(label)
                            .small()
                            .outline()
                            .disabled(installing)
                            .on_click(move |_, _, cx| {
                                let debugger = debugger.clone();
                                this.update(cx, |this, cx| this.install(debugger, cx)).ok();
                            }),
                    ),
                )
                .when_some(report.clone(), |this, (text, ok)| {
                    this.child(
                        div()
                            .text_sm()
                            .text_color(if ok {
                                cx.theme().muted_foreground
                            } else {
                                cx.theme().danger
                            })
                            .child(text),
                    )
                })
        })
        .keywords(["install", "download", "debug", "debugger"])
    }

    /// Run the debugger's install command in the background; turn it on
    /// once it's there.
    fn install(&mut self, debugger: Arc<Debugger>, cx: &mut Context<Self>) {
        if matches!(self.installs.get(&debugger.key), Some(Fetch::Running)) {
            return;
        }
        self.installs.insert(debugger.key.clone(), Fetch::Running);
        cx.spawn(async move |this, cx| {
            let installing = debugger.clone();
            let result = cx
                .background_executor()
                .spawn(async move { debuggers::install(&installing) })
                .await;
            this.update(cx, |this, cx| {
                let fetch = match result {
                    Ok(()) => {
                        settings::update(cx, |s| s.debugging.set_enabled(&debugger.key, true));
                        Fetch::Done(())
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

/// Marks an agent model OpenCode knows by itself, from an older file's
/// `agent_model`.
const OPENCODE_OWN: &str = "opencode:";

/// How the model lists name a provider.
fn provider_label(name: &str) -> &str {
    template_named(name).map_or(name, |template| template.label)
}

/// Every model the connected providers offer, as (`provider/model`, label).
fn offered_models(config: &Config, cx: &App) -> Vec<(SharedString, SharedString)> {
    let mut offered = Vec::new();
    for provider in &config.providers {
        if let Some(ModelList::Loaded(models)) = providers::models(&provider.name, cx) {
            for model in models {
                offered.push((
                    format!("{}/{model}", provider.name).into(),
                    format!("{} · {model}", provider_label(&provider.name)).into(),
                ));
            }
        }
    }
    offered
}

/// Keep the model in use in its list while models load, or if the
/// provider stopped offering it.
fn keep_current(options: &mut Vec<(SharedString, SharedString)>, id: &str, config: &Config) {
    if id.is_empty() || options.iter().any(|(value, _)| value == id) {
        return;
    }
    let label = match jig_ai::split_model(id) {
        Some((name, model)) if config.provider(name).is_some() => {
            format!("{} · {model}", provider_label(name))
        }
        _ => format!("{id} (not connected)"),
    };
    options.insert(0, (id.to_string().into(), label.into()));
}

/// What a provider row says, and whether it's fine.
fn connection_status(provider: &ProviderConfig, cx: &App) -> (String, bool) {
    if !providers::is_ready(provider, cx) {
        return ("Paste a key to connect".into(), false);
    }
    let source = providers::api_key(provider, cx)
        .map(|(_, source)| format!(" · {}", source.label()))
        .unwrap_or_default();
    match providers::models(&provider.name, cx) {
        Some(ModelList::Loaded(models)) => (format!("{} models{source}", models.len()), true),
        Some(ModelList::Failed(error)) => (error, false),
        Some(ModelList::Loading) | None => (format!("Loading models…{source}"), true),
    }
}

fn status_text(text: impl Into<SharedString>, ok: bool, cx: &App) -> impl IntoElement {
    div()
        .text_xs()
        .text_color(if ok {
            cx.theme().muted_foreground
        } else {
            cx.theme().danger
        })
        .child(text.into())
}

/// A provider that takes a key: the key field, and Disconnect once it's in
/// the file.
fn keyed_item(
    template: &'static ProviderTemplate,
    provider: Option<ProviderConfig>,
    input: Entity<InputState>,
    cx: &Context<SettingsWindow>,
) -> SettingItem {
    let this = cx.entity().downgrade();
    let env = template
        .api_key_env
        .filter(|var| std::env::var(var).is_ok_and(|key| !key.trim().is_empty()));
    SettingItem::new(
        template.label,
        SettingField::render(move |_, _, cx| {
            let status = provider
                .as_ref()
                .map(|provider| connection_status(provider, cx));
            let disconnect = this.clone();
            let use_env = this.clone();
            v_flex()
                .gap_1()
                .items_end()
                .child(Input::new(&input).small().mask_toggle().w_64())
                .child(
                    h_flex()
                        .gap_2()
                        .items_center()
                        .when_some(status, |row, (text, ok)| {
                            row.child(status_text(text, ok, cx))
                        })
                        .when_some(env.filter(|_| provider.is_none()), |row, var| {
                            row.child(
                                Button::new(SharedString::from(format!("env-{}", template.name)))
                                    .label(format!("Use {var}"))
                                    .xsmall()
                                    .ghost()
                                    .on_click(move |_, _, cx| {
                                        let result = providers::connect(template, None, cx);
                                        use_env.update(cx, |this, cx| this.report(result, cx)).ok();
                                    }),
                            )
                        })
                        .when(provider.is_some(), |row| {
                            row.child(
                                Button::new(SharedString::from(format!(
                                    "disconnect-{}",
                                    template.name
                                )))
                                .label("Disconnect")
                                .xsmall()
                                .ghost()
                                .on_click(move |_, _, cx| {
                                    let result = providers::disconnect(template.name, cx);
                                    disconnect
                                        .update(cx, |this, cx| this.report(result, cx))
                                        .ok();
                                }),
                            )
                        }),
                )
        }),
    )
    .keywords([template.name, "key", "api", "token", "connect"])
}

/// A server on this computer: a switch, since it needs no key.
fn local_item(
    template: &'static ProviderTemplate,
    provider: Option<ProviderConfig>,
    cx: &Context<SettingsWindow>,
) -> SettingItem {
    let this = cx.entity().downgrade();
    let description = match &provider {
        Some(provider) => connection_status(provider, cx).0,
        None => format!("Runs on this computer at {}", template.base_url),
    };
    SettingItem::new(
        template.label,
        SettingField::switch(
            move |cx| {
                providers::config(cx).is_ok_and(|config| config.provider(template.name).is_some())
            },
            move |on, cx| {
                let result = if on {
                    providers::connect(template, None, cx)
                } else {
                    providers::disconnect(template.name, cx)
                };
                this.update(cx, |this, cx| this.report(result, cx)).ok();
            },
        )
        .default_value(false),
    )
    .description(description)
    .keywords([template.name, "local", "connect"])
}

/// A provider added to `config.toml` by hand, which only the file can
/// change.
fn other_provider_item(provider: &ProviderConfig, cx: &Context<SettingsWindow>) -> SettingItem {
    let this = cx.entity().downgrade();
    let name = provider.name.clone();
    let (status, _) = connection_status(provider, cx);
    SettingItem::new(
        provider.name.clone(),
        SettingField::render(move |_, _, _| {
            let this = this.clone();
            let name = name.clone();
            h_flex().child(
                Button::new(SharedString::from(format!("remove-{name}")))
                    .label("Remove")
                    .small()
                    .ghost()
                    .on_click(move |_, _, cx| {
                        let result = providers::disconnect(&name, cx);
                        this.update(cx, |this, cx| this.report(result, cx)).ok();
                    }),
            )
        }),
    )
    .description(format!("{} · {status}", provider.base_url))
}

fn edit_providers_file() -> SettingItem {
    SettingItem::render(|_, _, _| {
        h_flex().child(
            Button::new("edit-config")
                .label("Edit Providers File")
                .small()
                .outline()
                .on_click(|_, _, cx| send_to_workspace(Box::new(EditModelConfig), cx)),
        )
    })
    .keywords(["config", "provider", "toml", "custom"])
}

impl Render for SettingsWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.sync_color_pickers(window, cx);
        let colors_page = self.colors_page(cx);
        let model_page = self.model_page(cx);
        let languages_page = self.languages_page(cx);
        let shortcuts_page = self.shortcuts_page(cx);
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
            .child(div().flex_1().min_h_0().child({
                let settings = Settings::new("jig-settings")
                    .sidebar_width(px(200.))
                    .sidebar_size_range(px(170.)..px(280.))
                    .with_group_variant(GroupBoxVariant::Outline)
                    .pages([
                        self.appearance_page(),
                        colors_page,
                        languages_page,
                        shortcuts_page,
                        self.commands_page(),
                        model_page,
                        self.agent_page(),
                    ]);
                #[cfg(test)]
                let settings =
                    settings.default_selected_index(gpui_kit::component::setting::SelectIndex {
                        page_ix: START_PAGE.get(),
                        group_ix: None,
                    });
                settings
            }))
    }
}

#[cfg(test)]
mod tests {
    use gpui_kit::component::ActiveTheme as _;
    use gpui_kit::{AppContext as _, TestAppContext};
    use jig_commands::Preset;

    use super::{
        CommandRow, Fetch, MAX_PROMPT_CHARS, OpenWindow, START_PAGE, SettingsWindow, init,
        is_settings_window, open, set_color,
    };
    use crate::providers;
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

    #[gpui_kit::test]
    fn the_agent_page_draws_with_every_choice(cx: &mut TestAppContext) {
        START_PAGE.set(6);
        cx.update(|cx| {
            gpui_kit::init(cx);
            crate::theme::init(cx);
            init(cx);
            open(cx);
        });
        cx.run_until_parked();
        let handle = cx.update(|cx| cx.global::<OpenWindow>().0);
        let draw = |cx: &mut TestAppContext| {
            cx.update_window(handle, |_, window, cx| window.draw(cx).clear(cx))
                .unwrap();
        };
        draw(cx);
        for access in jig_ai::agent::Access::ALL {
            cx.update(|cx| {
                settings::update(cx, |s| {
                    s.agent.shell = access;
                    s.agent.questions = !s.agent.questions;
                })
            });
            draw(cx);
        }
        assert_eq!(
            cx.update(|cx| settings::get(cx).agent.shell),
            jig_ai::agent::Access::Deny
        );
        START_PAGE.set(0);
    }

    #[gpui_kit::test]
    fn the_model_page_draws_in_every_state(cx: &mut TestAppContext) {
        let _dir = providers::init_temp(cx);
        START_PAGE.set(5);
        cx.update(|cx| {
            gpui_kit::init(cx);
            crate::theme::init(cx);
            init(cx);
            open(cx);
        });
        cx.run_until_parked();
        let handle = cx.update(|cx| cx.global::<OpenWindow>().0);
        let draw = |cx: &mut TestAppContext| {
            cx.update_window(handle, |_, window, cx| window.draw(cx).clear(cx))
                .unwrap();
        };
        let update = |cx: &mut TestAppContext, change: fn(&mut SettingsWindow)| {
            cx.update_window(handle, |root, _, cx| {
                let window = root.downcast::<gpui_kit::component::Root>().unwrap();
                let view = window.read(cx).view().clone().downcast::<SettingsWindow>();
                view.unwrap().update(cx, |this, cx| {
                    change(this);
                    cx.notify();
                });
            })
            .unwrap();
        };
        draw(cx);

        update(cx, |this| {
            this.check = Some(Fetch::Failed("401 Unauthorized".into()));
            this.providers_error = Some("Couldn't write config.toml.".into());
        });
        draw(cx);

        // Connected providers: one that takes a key, and a local one.
        cx.update(|cx| {
            let anthropic = jig_ai::template_named("anthropic").unwrap();
            providers::connect(anthropic, Some("sk-test"), cx).unwrap();
            providers::connect(jig_ai::template_named("ollama").unwrap(), None, cx).unwrap();
            providers::set_quick("anthropic/claude-haiku-4-5", cx).unwrap();
            providers::set_agent(Some("anthropic/claude-sonnet-5-5"), cx).unwrap();
        });
        draw(cx);
        cx.update(|cx| providers::disconnect("opencode-go", cx).unwrap());
        draw(cx);
        START_PAGE.set(0);
    }

    #[gpui_kit::test]
    fn records_a_shortcut_and_resets_it(cx: &mut TestAppContext) {
        START_PAGE.set(3);
        cx.update(|cx| {
            gpui_kit::init(cx);
            crate::theme::init(cx);
            init(cx);
            open(cx);
        });
        cx.run_until_parked();
        let handle = cx.update(|cx| cx.global::<OpenWindow>().0);
        let draw = |cx: &mut TestAppContext| {
            cx.update_window(handle, |_, window, cx| window.draw(cx).clear(cx))
                .unwrap();
        };
        draw(cx);
        cx.update_window(handle, |root, window, cx| {
            let root = root.downcast::<gpui_kit::component::Root>().unwrap();
            let view = root.read(cx).view().clone().downcast::<SettingsWindow>();
            view.unwrap()
                .update(cx, |this, cx| this.record("save", window, cx));
        })
        .unwrap();
        cx.simulate_keystrokes(handle, "secondary-shift-u");
        let expected = gpui_kit::Keystroke::parse("secondary-shift-u")
            .unwrap()
            .unparse();
        assert_eq!(
            cx.update(|cx| settings::get(cx).shortcuts.get("save").cloned()),
            Some(expected)
        );
        draw(cx);

        // Pressing the default again forgets the change.
        cx.update_window(handle, |root, window, cx| {
            let root = root.downcast::<gpui_kit::component::Root>().unwrap();
            let view = root.read(cx).view().clone().downcast::<SettingsWindow>();
            view.unwrap()
                .update(cx, |this, cx| this.record("save", window, cx));
        })
        .unwrap();
        cx.simulate_keystrokes(handle, "secondary-s");
        assert!(cx.update(|cx| settings::get(cx).shortcuts.is_empty()));
        START_PAGE.set(0);
    }
}
