//! Appearance (theme, code font, translucency), its Theme Colors page, and
//! Editor (what the code view shows, typing helps, and what saving does).

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::button::{Toggle, ToggleGroup, ToggleVariants as _};
use gpui_kit::component::color_picker::ColorPicker;
use gpui_kit::component::{ActiveTheme as _, IconName, Sizable as _, h_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::ui::{self, Row, Section};
use super::{Detail, Nav, Page, SettingsWindow};
use crate::settings::{
    self, DEFAULT_TRANSLUCENCY, EditorSettings, FormatSettings, MAX_FONT_SIZE, MAX_TRANSLUCENCY,
    MIN_FONT_SIZE, ThemeChoice,
};
use crate::theme::{self, EDITABLE, Section as ColorSection};

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
pub(super) fn set_color(key: &str, color: Option<Hsla>, cx: &mut App) {
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

/// The theme in use, by name.
fn variant_name(cx: &App) -> &'static str {
    if cx.theme().is_dark() {
        "Ember"
    } else {
        "Ember Light"
    }
}

impl SettingsWindow {
    pub(super) fn appearance_sections(&self, cx: &Context<Self>) -> Vec<Section> {
        let changed = settings::get(cx).colors.variant(cx.theme().is_dark()).len();
        let colors_value = match changed {
            0 => variant_name(cx).to_string(),
            1 => "1 change".to_string(),
            n => format!("{n} changes"),
        };
        let mut theme = Section::new()
            .row(
                Row::new("appearance", "Appearance")
                    .keywords(["theme", "dark", "light", "mode", "system"])
                    .control(|_, cx| theme_picker(cx)),
            )
            .row(
                Row::new("colors", "Theme colors")
                    .keywords(["color", "colour", "syntax", "highlight", "scheme"])
                    .value(colors_value)
                    .opens(self.opener(Nav::detail(Page::Appearance, Detail::Colors), cx)),
            )
            .row(
                Row::new("font-size", "Code font size")
                    .description("In points. ⌘= and ⌘- change it too.")
                    .keywords(["font", "size", "zoom", "text"])
                    .control(ui::number_input(
                        "font-size",
                        (MIN_FONT_SIZE.into(), MAX_FONT_SIZE.into(), 0.5),
                        |cx| settings::get(cx).appearance.font_size.into(),
                        |size, cx| settings::update(cx, |s| s.appearance.font_size = size as f32),
                    )),
            );
        // Only macOS blurs what is behind the window.
        if cfg!(target_os = "macos") {
            theme = theme.row(
                Row::new("translucency", "Translucency")
                    .description(format!(
                        "How much of the desktop shows through, in percent. {} by default.",
                        DEFAULT_TRANSLUCENCY
                    ))
                    .keywords(["transparent", "blur", "vibrancy", "window"])
                    .control(ui::number_input(
                        "translucency",
                        (0., MAX_TRANSLUCENCY.into(), 5.),
                        |cx| settings::get(cx).appearance.translucency.into(),
                        |percent, cx| {
                            settings::update(cx, |s| s.appearance.translucency = percent as f32)
                        },
                    )),
            );
        }
        vec![theme]
    }

    /// Show each picker the color in use, unless it's being picked.
    pub(super) fn sync_color_pickers(&self, window: &mut Window, cx: &mut App) {
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

    pub(super) fn colors_sections(&self, cx: &Context<Self>) -> Vec<Section> {
        let variant = variant_name(cx);
        let any_changed = !settings::get(cx)
            .colors
            .variant(cx.theme().is_dark())
            .is_empty();
        let rows = |section: ColorSection| {
            EDITABLE
                .iter()
                .zip(&self.color_pickers)
                .filter(move |(editable, _)| editable.section == section)
                .map(|(editable, picker)| {
                    let key = editable.key;
                    let picker = picker.clone();
                    Row::new(key, editable.label)
                        .keywords([key, "color", "colour"])
                        .control(move |_, cx| {
                            let hex = color_in_use(key, cx).map(theme::to_hex).unwrap_or_default();
                            h_flex()
                                .gap_2()
                                .items_center()
                                .when(is_changed(key, cx), |this| {
                                    this.child(
                                        Button::new(SharedString::from(format!("reset-{key}")))
                                            .icon(IconName::Undo2)
                                            .ghost()
                                            .xsmall()
                                            .tooltip("Back to Ember's")
                                            .on_click(move |_, _, cx| set_color(key, None, cx)),
                                    )
                                })
                                .child(
                                    div()
                                        .text_size(px(ui::SMALL))
                                        .font_family(cx.theme().mono_font_family.clone())
                                        .text_color(cx.theme().muted_foreground)
                                        .child(hex),
                                )
                                .child(ColorPicker::new(&picker).small().anchor(Anchor::TopRight))
                        })
                })
        };
        let mut code = Section::titled("Code")
            .footer(format!(
                "Changes apply to {variant}, the theme in use. Switch Appearance to change \
                 the other."
            ))
            .rows(rows(ColorSection::Code));
        if any_changed {
            code = code.accessory(move |_, _| {
                ui::button("reset-colors", format!("Reset {variant}")).on_click(|_, _, cx| {
                    let dark = cx.theme().is_dark();
                    settings::update(cx, |s| s.colors.variant_mut(dark).clear())
                })
            });
        }
        vec![
            code,
            Section::titled("Editor").rows(rows(ColorSection::Editor)),
            Section::titled("Interface").rows(rows(ColorSection::Interface)),
        ]
    }
}

/// System, Light or Dark, side by side.
fn theme_picker(cx: &App) -> ToggleGroup {
    let current = settings::get(cx).appearance.theme;
    ToggleGroup::new("theme")
        .segmented()
        .outline()
        .small()
        .children(ThemeChoice::ALL.into_iter().map(|choice| {
            Toggle::new(SharedString::from(choice.key()))
                .label(choice.label())
                .checked(choice == current)
        }))
        .on_click(move |checks: &Vec<bool>, _, cx| {
            // The one clicked is the one whose state changed.
            let clicked = ThemeChoice::ALL
                .into_iter()
                .zip(checks)
                .find(|(choice, checked)| (*choice == current) != **checked)
                .map(|(choice, _)| choice);
            if let Some(choice) = clicked {
                settings::update(cx, |s| s.appearance.theme = choice);
            }
        })
}

fn editor_switch(
    id: &'static str,
    label: &'static str,
    get: fn(&EditorSettings) -> bool,
    set: fn(&mut EditorSettings, bool),
) -> Row {
    Row::new(id, label).control(ui::switch(
        id,
        move |cx| get(&settings::get(cx).editor),
        move |on, cx| settings::update(cx, |s| set(&mut s.editor, on)),
    ))
}

fn save_switch(
    id: &'static str,
    label: &'static str,
    get: fn(&FormatSettings) -> bool,
    set: fn(&mut FormatSettings, bool),
) -> Row {
    Row::new(id, label)
        .keywords(["save", "format", "whitespace", "newline"])
        .control(ui::switch(
            id,
            move |cx| get(&settings::get(cx).formatting),
            move |on, cx| settings::update(cx, |s| set(&mut s.formatting, on)),
        ))
}

pub(super) fn editor_sections() -> Vec<Section> {
    vec![
        Section::titled("Display")
            .row(
                editor_switch(
                    "line-numbers",
                    "Line numbers",
                    |e| e.line_numbers,
                    |e, on| e.line_numbers = on,
                )
                .keywords(["gutter"]),
            )
            .row(
                editor_switch(
                    "soft-wrap",
                    "Wrap long lines",
                    |e| e.soft_wrap,
                    |e, on| e.soft_wrap = on,
                )
                .description("At the window's edge, instead of scrolling sideways.")
                .keywords(["soft wrap", "word wrap"]),
            )
            .row(editor_switch(
                "indent-guides",
                "Indent guides",
                |e| e.indent_guides,
                |e, on| e.indent_guides = on,
            ))
            .row(
                editor_switch(
                    "whitespace",
                    "Show whitespace",
                    |e| e.show_whitespace,
                    |e, on| e.show_whitespace = on,
                )
                .description("Mark spaces and tabs.")
                .keywords(["invisibles", "tabs", "spaces"]),
            ),
        Section::titled("Typing")
            .row(
                editor_switch(
                    "autocomplete",
                    "Autocomplete",
                    |e| e.autocomplete,
                    |e, on| e.autocomplete = on,
                )
                .description("Suggest names from the language server, or words in the file.")
                .keywords(["completion", "suggest", "intellisense"]),
            )
            .row(
                editor_switch(
                    "selection-button",
                    "Jig button on selections",
                    |e| e.selection_button,
                    |e, on| e.selection_button = on,
                )
                .description("A button beside selected code that opens ⌘K.")
                .keywords(["sparkle", "selection", "jig"]),
            ),
        Section::titled("On Save")
            .footer("Formatting on save is set per language, under Languages.")
            .row(save_switch(
                "trim-whitespace",
                "Trim trailing whitespace",
                |f| f.trim_trailing_whitespace,
                |f, on| f.trim_trailing_whitespace = on,
            ))
            .row(save_switch(
                "final-newline",
                "End files with a newline",
                |f| f.final_newline,
                |f, on| f.final_newline = on,
            )),
    ]
}
