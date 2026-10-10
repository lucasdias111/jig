//! `~/.config/jig/settings.toml`: the choices made in the Settings window.
//! Held in a global while Jig runs; every change is written back at once.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};
use gpui_kit::{App, Global};
use serde::{Deserialize, Serialize};

pub const DEFAULT_FONT_SIZE: f32 = 13.5;
pub const MIN_FONT_SIZE: f32 = 9.;
pub const MAX_FONT_SIZE: f32 = 28.;
pub const DEFAULT_TRANSLUCENCY: f32 = 20.;
pub const MAX_TRANSLUCENCY: f32 = 40.;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub appearance: Appearance,
    pub editor: EditorSettings,
    /// Written as `jigs`; files from before the rename say `commands`.
    #[serde(rename = "jigs", alias = "commands")]
    pub commands: CommandSettings,
    pub languages: LanguageSettings,
    pub debugging: DebugSettings,
    pub formatting: FormatSettings,
    /// What the agent may do without asking, or at all.
    pub agent: jig_ai::agent::Permissions,
    pub colors: ColorSettings,
    /// Changed keyboard shortcuts, by name: GPUI keystrokes, `""` for none.
    pub shortcuts: BTreeMap<String, String>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemeChoice {
    /// Light or dark, following the system.
    #[default]
    System,
    Light,
    Dark,
}

impl ThemeChoice {
    pub const ALL: [ThemeChoice; 3] = [ThemeChoice::System, ThemeChoice::Light, ThemeChoice::Dark];

    pub fn key(self) -> &'static str {
        match self {
            ThemeChoice::System => "system",
            ThemeChoice::Light => "light",
            ThemeChoice::Dark => "dark",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            ThemeChoice::System => "System",
            ThemeChoice::Light => "Light",
            ThemeChoice::Dark => "Dark",
        }
    }

    pub fn from_key(key: &str) -> Self {
        Self::ALL
            .into_iter()
            .find(|choice| choice.key() == key)
            .unwrap_or_default()
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Appearance {
    pub theme: ThemeChoice,
    /// Size of the code font, in points.
    pub font_size: f32,
    /// How much of the blurred desktop shows through the editor, in
    /// percent. Only macOS blurs the window, so elsewhere it is ignored.
    pub translucency: f32,
}

impl Default for Appearance {
    fn default() -> Self {
        Self {
            theme: ThemeChoice::System,
            font_size: DEFAULT_FONT_SIZE,
            translucency: DEFAULT_TRANSLUCENCY,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct EditorSettings {
    pub line_numbers: bool,
    pub soft_wrap: bool,
    pub indent_guides: bool,
    pub show_whitespace: bool,
    /// Suggest completions while typing.
    pub autocomplete: bool,
    /// Show the button beside a selection that opens the command input.
    pub selection_button: bool,
}

impl Default for EditorSettings {
    fn default() -> Self {
        Self {
            line_numbers: true,
            soft_wrap: true,
            indent_guides: true,
            show_whitespace: false,
            autocomplete: true,
            selection_button: true,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct CommandSettings {
    /// Commands left out of the command input, by name.
    pub hidden: Vec<String>,
}

impl CommandSettings {
    pub fn is_hidden(&self, name: &str) -> bool {
        self.hidden
            .iter()
            .any(|hidden| hidden.eq_ignore_ascii_case(name))
    }

    pub fn set_hidden(&mut self, name: &str, hidden: bool) {
        self.hidden
            .retain(|other| !other.eq_ignore_ascii_case(name));
        if hidden {
            self.hidden.push(name.to_string());
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct LanguageSettings {
    /// Languages left unhighlighted, by name.
    pub off: Vec<String>,
    /// File extensions per language, as typed, replacing the built-in ones:
    /// `typescript = "ts, mts"`.
    pub extensions: BTreeMap<String, String>,
}

impl LanguageSettings {
    pub fn is_off(&self, name: &str) -> bool {
        self.off.iter().any(|off| off == name)
    }

    pub fn set_off(&mut self, name: &str, off: bool) {
        self.off.retain(|other| other != name);
        if off {
            self.off.push(name.to_string());
        }
    }

    /// Set a language's extensions; `None`, or the built-in ones, clears the
    /// override so a later change to the built-ins applies.
    pub fn set_extensions(&mut self, name: &str, text: Option<String>, built_in: &str) {
        match text.filter(|text| text != built_in) {
            Some(text) => self.extensions.insert(name.to_string(), text),
            None => self.extensions.remove(name),
        };
    }
}

/// Which debuggers are on, by key (`rust`, `typescript`, or one of the
/// user's in `debuggers.toml`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct DebugSettings {
    pub enabled: Vec<String>,
}

impl Default for DebugSettings {
    /// Rust's debugger comes with Xcode, so it's on to begin with.
    fn default() -> Self {
        Self {
            enabled: vec!["rust".into()],
        }
    }
}

impl DebugSettings {
    pub fn is_enabled(&self, key: &str) -> bool {
        self.enabled.iter().any(|on| on == key)
    }

    pub fn set_enabled(&mut self, key: &str, on: bool) {
        self.enabled.retain(|other| other != key);
        if on {
            self.enabled.push(key.to_string());
        }
    }
}

/// What happens to a file as it's saved. All off to begin with: nothing
/// changes what you saved unless you asked.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct FormatSettings {
    /// Languages formatted on save, by name.
    pub on_save: Vec<String>,
    pub trim_trailing_whitespace: bool,
    pub final_newline: bool,
}

impl FormatSettings {
    pub fn on_save(&self, language: &str) -> bool {
        self.on_save.iter().any(|on| on == language)
    }

    pub fn set_on_save(&mut self, language: &str, on: bool) {
        self.on_save.retain(|other| other != language);
        if on {
            self.on_save.push(language.to_string());
        }
    }
}

/// Colors changed in the theme editor, over Ember's own, per variant:
/// `[colors.dark]` with `"syntax.keyword" = "#FF6188"`. Keys are listed in
/// `theme::EDITABLE`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ColorSettings {
    pub light: BTreeMap<String, String>,
    pub dark: BTreeMap<String, String>,
}

impl ColorSettings {
    pub fn variant(&self, dark: bool) -> &BTreeMap<String, String> {
        if dark { &self.dark } else { &self.light }
    }

    pub fn variant_mut(&mut self, dark: bool) -> &mut BTreeMap<String, String> {
        if dark {
            &mut self.dark
        } else {
            &mut self.light
        }
    }
}

impl Settings {
    /// `~/.config/jig/settings.toml`, honouring `XDG_CONFIG_HOME`.
    pub fn user_path() -> Option<PathBuf> {
        let config = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))?;
        Some(config.join("jig").join("settings.toml"))
    }

    /// The saved settings, or the defaults when there is no file yet.
    pub fn load(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let source =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        let mut settings: Settings =
            toml::from_str(&source).with_context(|| format!("parsing {}", path.display()))?;
        settings.appearance.font_size = settings
            .appearance
            .font_size
            .clamp(MIN_FONT_SIZE, MAX_FONT_SIZE);
        settings.appearance.translucency =
            settings.appearance.translucency.clamp(0., MAX_TRANSLUCENCY);
        Ok(settings)
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        }
        let source = toml::to_string_pretty(self)?;
        std::fs::write(path, source).with_context(|| format!("writing {}", path.display()))
    }
}

/// The live settings and where they're saved.
pub struct AppSettings {
    settings: Settings,
    /// `None` keeps changes in memory only, as in tests.
    path: Option<PathBuf>,
}

impl Global for AppSettings {}

/// Load the user's settings into the app. Returns why they couldn't be read,
/// in which case the defaults are used and nothing is saved over the file.
pub fn init(cx: &mut App) -> Option<String> {
    let path = Settings::user_path();
    let (settings, path, error) = match path.as_deref().map(Settings::load) {
        Some(Ok(settings)) => (settings, path, None),
        Some(Err(error)) => (Settings::default(), None, Some(format!("{error:#}"))),
        None => (Settings::default(), None, None),
    };
    cx.set_global(AppSettings { settings, path });
    error
}

/// The current settings; the defaults until `init` has run.
pub fn get(cx: &App) -> Settings {
    cx.try_global::<AppSettings>()
        .map(|app| app.settings.clone())
        .unwrap_or_default()
}

/// Change the settings, save them, and notify everything observing them.
pub fn update(cx: &mut App, change: impl FnOnce(&mut Settings)) {
    if !cx.has_global::<AppSettings>() {
        cx.set_global(AppSettings {
            settings: Settings::default(),
            path: None,
        });
    }
    let app = cx.global_mut::<AppSettings>();
    let before = app.settings.clone();
    change(&mut app.settings);
    if app.settings == before {
        return;
    }
    if let Some(path) = &app.path
        && let Err(error) = app.settings.save(path)
    {
        eprintln!("jig: settings weren't saved: {error:#}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_file_gives_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let settings = Settings::load(&dir.path().join("settings.toml")).unwrap();
        assert_eq!(settings, Settings::default());
        assert_eq!(settings.appearance.font_size, DEFAULT_FONT_SIZE);
        assert!(settings.editor.line_numbers);
    }

    #[test]
    fn round_trips_and_fills_in_missing_keys() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("jig").join("settings.toml");
        let mut settings = Settings::default();
        settings.appearance.theme = ThemeChoice::Dark;
        settings.editor.soft_wrap = false;
        settings.commands.set_hidden("Explain", true);
        settings.languages.set_off("java", true);
        settings.formatting.set_on_save("rust", true);
        settings.formatting.final_newline = true;
        settings.agent.shell = jig_ai::agent::Access::Allow;
        settings.agent.questions = false;
        settings
            .languages
            .set_extensions("typescript", Some("ts".into()), "ts, mts, cts");
        settings
            .colors
            .dark
            .insert("syntax.keyword".into(), "#FF6188".into());
        settings.save(&path).unwrap();
        assert_eq!(Settings::load(&path).unwrap(), settings);

        std::fs::write(
            &path,
            "[appearance]\ntheme = \"light\"\nfont_size = 99\ntranslucency = 90\n",
        )
        .unwrap();
        let partial = Settings::load(&path).unwrap();
        assert_eq!(partial.appearance.theme, ThemeChoice::Light);
        assert_eq!(partial.appearance.font_size, MAX_FONT_SIZE, "clamped");
        assert_eq!(partial.appearance.translucency, MAX_TRANSLUCENCY, "clamped");
        assert_eq!(partial.editor, EditorSettings::default());
        assert_eq!(partial.formatting, FormatSettings::default(), "off");
        assert_eq!(
            partial.agent,
            jig_ai::agent::Permissions::default(),
            "everything asks"
        );
    }

    #[test]
    fn hidden_commands_match_any_case() {
        let mut commands = CommandSettings::default();
        commands.set_hidden("Add docs", true);
        assert!(commands.is_hidden("add DOCS"));
        commands.set_hidden("ADD DOCS", true);
        assert_eq!(commands.hidden.len(), 1);
        commands.set_hidden("add docs", false);
        assert!(!commands.is_hidden("Add docs"));
    }

    #[test]
    fn built_in_extensions_clear_the_override() {
        let mut languages = LanguageSettings::default();
        languages.set_extensions("go", Some("go, gox".into()), "go");
        assert_eq!(languages.extensions["go"], "go, gox");
        languages.set_extensions("go", Some("go".into()), "go");
        assert!(languages.extensions.is_empty());
        languages.set_off("go", true);
        languages.set_off("go", true);
        assert_eq!(languages.off, ["go"]);
    }

    #[test]
    fn bad_file_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.toml");
        std::fs::write(&path, "[appearance]\ntheme = \"purple\"\n").unwrap();
        assert!(Settings::load(&path).is_err());
    }
}
