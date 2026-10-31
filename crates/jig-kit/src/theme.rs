//! Jig's look: Ember, in a light and a dark variant that follow the
//! system unless Settings picks one. Colors changed in Settings are laid
//! over the bundled ones.

use std::rc::Rc;
use std::sync::LazyLock;

use gpui_kit::component::highlighter::HighlightThemeStyle;
use gpui_kit::component::{Theme, ThemeConfig, ThemeMode, ThemeSet};
use gpui_kit::{App, Global, Hsla, Rgba, SharedString, Window};
use jig_commands::surface::EditorOpacity;
use serde_json::Value;

use jig_settings::{self as settings, ColorSettings, ThemeChoice};

const EMBER: &str = include_str!("../../../assets/themes/ember.json");

static BUNDLED: LazyLock<Value> =
    LazyLock::new(|| serde_json::from_str(EMBER).expect("bundled ember.json is valid"));

/// Where an editable color shows up, for grouping in Settings.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Section {
    Code,
    Editor,
    Interface,
}

/// A color the user can change. `syntax.*` keys are kinds of code,
/// `editor.*` keys belong to the editor's highlight theme, and the rest are
/// interface colors, named as in the theme file.
pub struct Editable {
    pub key: &'static str,
    pub label: &'static str,
    pub section: Section,
}

const fn editable(key: &'static str, label: &'static str, section: Section) -> Editable {
    Editable {
        key,
        label,
        section,
    }
}

pub const EDITABLE: &[Editable] = &[
    editable("syntax.keyword", "Keyword", Section::Code),
    editable("syntax.function", "Function", Section::Code),
    editable("syntax.type", "Type", Section::Code),
    editable("syntax.constructor", "Constructor", Section::Code),
    editable("syntax.variant", "Enum variant", Section::Code),
    editable("syntax.property", "Property", Section::Code),
    editable("syntax.variable", "Variable", Section::Code),
    editable(
        "syntax.variable.special",
        "Special variable (self)",
        Section::Code,
    ),
    editable("syntax.string", "String", Section::Code),
    editable("syntax.string.escape", "Escape sequence", Section::Code),
    editable("syntax.number", "Number", Section::Code),
    editable("syntax.boolean", "Boolean", Section::Code),
    editable("syntax.constant", "Constant", Section::Code),
    editable("syntax.comment", "Comment", Section::Code),
    editable("syntax.comment.doc", "Doc comment", Section::Code),
    editable("syntax.attribute", "Attribute", Section::Code),
    editable("syntax.preproc", "Macro and preprocessor", Section::Code),
    editable("syntax.operator", "Operator", Section::Code),
    editable("syntax.punctuation", "Punctuation", Section::Code),
    editable("syntax.tag", "Tag", Section::Code),
    editable("syntax.label", "Label and lifetime", Section::Code),
    editable("syntax.title", "Heading", Section::Code),
    editable("syntax.link_text", "Link", Section::Code),
    editable("editor.background", "Background", Section::Editor),
    editable("editor.foreground", "Text", Section::Editor),
    editable(
        "editor.active_line.background",
        "Current line",
        Section::Editor,
    ),
    editable("editor.line_number", "Line number", Section::Editor),
    editable(
        "editor.active_line_number",
        "Current line number",
        Section::Editor,
    ),
    editable("caret", "Caret", Section::Editor),
    editable("selection.background", "Selection", Section::Editor),
    editable("background", "Background", Section::Interface),
    editable("foreground", "Text", Section::Interface),
    editable("muted.foreground", "Secondary text", Section::Interface),
    editable("primary.background", "Accent", Section::Interface),
    editable("info.background", "Agent", Section::Interface),
    editable(
        "success.background",
        "Success, Run and Debug",
        Section::Interface,
    ),
    editable("sidebar.background", "Sidebar", Section::Interface),
    editable(
        "sidebar.primary.background",
        "Sidebar selection",
        Section::Interface,
    ),
    editable("border", "Border", Section::Interface),
    editable("popover.background", "Popover", Section::Interface),
];

/// The bundled variant's own value for an editable color.
pub fn default_color(key: &str, dark: bool) -> Option<Hsla> {
    let theme = bundled_variant(dark)?;
    let value = match key.strip_prefix("syntax.") {
        Some(name) => &theme["highlight"]["syntax"][name]["color"],
        None if key.starts_with("editor.") => &theme["highlight"][key],
        None => &theme["colors"][key],
    };
    parse_hex(value.as_str()?)
}

fn bundled_variant(dark: bool) -> Option<&'static Value> {
    BUNDLED["themes"]
        .as_array()?
        .iter()
        .find(|theme| (theme["mode"] == "dark") == dark)
}

pub fn parse_hex(hex: &str) -> Option<Hsla> {
    let digits = hex.strip_prefix('#')?;
    if !matches!(digits.len(), 6 | 8) || !digits.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    Rgba::try_from(hex).ok().map(Hsla::from)
}

/// `#RRGGBB`, or `#RRGGBBAA` when not opaque.
pub fn to_hex(color: Hsla) -> String {
    let rgba = Rgba::from(color);
    let byte = |channel: f32| (channel.clamp(0., 1.) * 255.).round() as u8;
    let (r, g, b, a) = (byte(rgba.r), byte(rgba.g), byte(rgba.b), byte(rgba.a));
    if a == 255 {
        format!("#{r:02X}{g:02X}{b:02X}")
    } else {
        format!("#{r:02X}{g:02X}{b:02X}{a:02X}")
    }
}

/// Ember with the user's colors laid over it. Unknown keys and values that
/// aren't hex colors are skipped.
fn ember_with(colors: &ColorSettings) -> ThemeSet {
    let mut set = BUNDLED.clone();
    for theme in set["themes"].as_array_mut().into_iter().flatten() {
        let dark = theme["mode"] == "dark";
        for (key, hex) in colors.variant(dark) {
            if !EDITABLE.iter().any(|e| e.key == key) || parse_hex(hex).is_none() {
                continue;
            }
            let slot = match key.strip_prefix("syntax.") {
                Some(name) => &mut theme["highlight"]["syntax"][name]["color"],
                None if key.starts_with("editor.") => &mut theme["highlight"][key.as_str()],
                None => &mut theme["colors"][key.as_str()],
            };
            *slot = Value::from(hex.as_str());
        }
    }
    serde_json::from_value(set).expect("Ember with valid hex colors is a valid theme")
}

/// What the themes in use were built from.
struct Built {
    fonts: Fonts,
    colors: ColorSettings,
    see_through: SeeThrough,
}

/// How the editor lets the window's blur through.
#[derive(Clone, Copy, PartialEq)]
struct SeeThrough {
    /// Of the editor's surfaces, from 0 to 1.
    opacity: f32,
    /// Whether lines wrap. Then no code scrolls under the gutter, so it
    /// needs no background of its own.
    wraps: bool,
}

impl SeeThrough {
    fn from_settings(cx: &App) -> Self {
        Self {
            opacity: editor_opacity(cx),
            wraps: settings::get(cx).editor.soft_wrap,
        }
    }
}

impl Global for Built {}

/// Make Ember the light and dark theme, and keep it in step with the
/// settings. Call once, after `gpui_kit::init` and `settings::init`.
pub fn init(cx: &mut App) {
    let fonts = Fonts::for_platform(cx);
    let colors = settings::get(cx).colors;
    let see_through = SeeThrough::from_settings(cx);
    install(&fonts, &colors, see_through, cx);
    cx.set_global(Built {
        fonts,
        colors,
        see_through,
    });
    cx.set_global(EditorOpacity(see_through.opacity));
    cx.observe_global::<settings::AppSettings>(|cx| {
        let colors = settings::get(cx).colors;
        let see_through = SeeThrough::from_settings(cx);
        let built = cx.global::<Built>();
        if built.colors != colors || built.see_through != see_through {
            let fonts = built.fonts.clone();
            install(&fonts, &colors, see_through, cx);
            let built = cx.global_mut::<Built>();
            built.colors = colors;
            built.see_through = see_through;
            cx.set_global(EditorOpacity(see_through.opacity));
        }
        apply(None, cx)
    })
    .detach();
}

/// How opaque the editor's surfaces are. Only macOS blurs what is behind
/// the window; elsewhere a see-through editor would show the desktop
/// sharp, so it stays solid.
fn editor_opacity(cx: &App) -> f32 {
    if cfg!(target_os = "macos") {
        1. - settings::get(cx).appearance.translucency / 100.
    } else {
        1.
    }
}

pub use jig_commands::surface::editor_surface;

/// Build both variants and make them the themes for light and dark mode.
/// They take effect at the next `apply`.
fn install(fonts: &Fonts, colors: &ColorSettings, see_through: SeeThrough, cx: &mut App) {
    let font_size = Some(settings::get(cx).appearance.font_size);
    let mut light: Option<Rc<ThemeConfig>> = None;
    let mut dark = None;
    for mut theme in ember_with(colors).themes {
        theme.font_family = fonts.ui.clone();
        theme.mono_font_family = Some(fonts.mono.clone());
        theme.mono_font_size = font_size;
        if see_through.opacity < 1.
            && let Some(highlight) = theme.highlight.as_mut()
        {
            let foreground = highlight
                .editor_foreground
                .or_else(|| theme.colors.foreground.as_deref().and_then(parse_hex));
            see_through_editor(highlight, foreground, see_through);
        }
        let slot = if theme.mode.is_dark() {
            &mut dark
        } else {
            &mut light
        };
        *slot = Some(Rc::new(theme));
    }
    Theme::update(cx, |theme| {
        theme.light_theme = light.expect("ember.json has a light theme");
        theme.dark_theme = dark.expect("ember.json has a dark theme");
    });
}

/// The editor paints its gutter and current line over the surface behind
/// it. In their solid colors they would stand out as opaque bands.
fn see_through_editor(
    highlight: &mut HighlightThemeStyle,
    foreground: Option<Hsla>,
    see_through: SeeThrough,
) {
    let background = highlight.editor_background;
    // The current line becomes a faint wash of the text color, as far
    // from the background as the solid color was.
    if let (Some(line), Some(background), Some(foreground)) =
        (highlight.editor_active_line, background, foreground)
    {
        let distance = (foreground.l - background.l).abs().max(f32::EPSILON);
        let wash = ((line.l - background.l).abs() / distance).clamp(0., 1.);
        highlight.editor_active_line = Some(foreground.opacity(wash));
    }
    // The gutter covers code scrolled sideways under it, so without wrap it
    // keeps a background, at the surface's opacity.
    highlight.editor_gutter_background = if see_through.wraps {
        Some(gpui_kit::transparent_black())
    } else {
        background.map(|color| color.opacity(see_through.opacity))
    };
    highlight.editor_background = background.map(|color| color.opacity(see_through.opacity));
}

#[derive(Clone)]
struct Fonts {
    /// `None` keeps the platform UI font (SF Pro on macOS).
    ui: Option<SharedString>,
    mono: SharedString,
}

impl Fonts {
    fn for_platform(cx: &App) -> Self {
        if cfg!(target_os = "macos") {
            // The system's SF Mono. It isn't listed as a public family, but
            // Core Text resolves this name.
            return Self {
                ui: None,
                mono: ".AppleSystemUIFontMonospaced".into(),
            };
        }
        let installed = cx.text_system().all_font_names();
        let first = |candidates: &[&str]| {
            candidates
                .iter()
                .find(|name| installed.iter().any(|font| font == *name))
                .map(|name| SharedString::from(name.to_string()))
        };
        Self {
            ui: first(&["Inter"]),
            mono: first(&["JetBrains Mono", "DejaVu Sans Mono"])
                .unwrap_or_else(|| "monospace".into()),
        }
    }
}

/// Apply the variant the settings ask for: the one matching the window's
/// appearance, or a fixed one.
pub fn sync(window: &mut Window, cx: &mut App) {
    apply(Some(window), cx);
}

/// Apply the theme and code font size from the settings.
pub fn apply(window: Option<&mut Window>, cx: &mut App) {
    let appearance = settings::get(cx).appearance;
    let size = Some(appearance.font_size);
    let theme = Theme::global(cx);
    if theme.light_theme.mono_font_size != size || theme.dark_theme.mono_font_size != size {
        Theme::update(cx, |theme| {
            Rc::make_mut(&mut theme.light_theme).mono_font_size = size;
            Rc::make_mut(&mut theme.dark_theme).mono_font_size = size;
        });
    }
    // Both reload the mode's theme, so the new font size takes effect.
    match appearance.theme {
        ThemeChoice::System => Theme::sync_system_appearance(window, cx),
        ThemeChoice::Light => Theme::change(ThemeMode::Light, window, cx),
        ThemeChoice::Dark => Theme::change(ThemeMode::Dark, window, cx),
    }
}

#[cfg(test)]
mod tests;
