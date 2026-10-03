//! Jig's look: OneNord, in a light and a dark variant that follow the
//! system unless Settings picks one.

use std::rc::Rc;

use gpui_kit::component::{Theme, ThemeMode, ThemeSet};
use gpui_kit::{App, SharedString, Window};

use crate::settings::{self, ThemeChoice};

const ONENORD: &str = include_str!("../../../assets/themes/onenord.json");

/// Make OneNord the light and dark theme, and keep it in step with the
/// settings. Call once, after `gpui_kit::init` and `settings::init`.
pub fn init(cx: &mut App) {
    let set: ThemeSet = serde_json::from_str(ONENORD).expect("bundled onenord.json is valid");
    let mut light = None;
    let mut dark = None;
    let fonts = Fonts::for_platform(cx);
    for mut theme in set.themes {
        theme.font_family = fonts.ui.clone();
        theme.mono_font_family = Some(fonts.mono.clone());
        theme.mono_font_size = Some(settings::get(cx).appearance.font_size);
        let slot = if theme.mode.is_dark() {
            &mut dark
        } else {
            &mut light
        };
        *slot = Some(Rc::new(theme));
    }
    Theme::update(cx, |theme| {
        theme.light_theme = light.expect("onenord.json has a light theme");
        theme.dark_theme = dark.expect("onenord.json has a dark theme");
    });
    cx.observe_global::<settings::AppSettings>(|cx| apply(None, cx))
        .detach();
}

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
mod tests {
    use gpui_kit::component::{Theme, ThemeMode, ThemeSet};
    use gpui_kit::{TestAppContext, rgb};

    use super::{ONENORD, apply, init};
    use crate::settings::{self, ThemeChoice};

    #[gpui_kit::test]
    fn onenord_applies_in_both_modes(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            init(cx);

            Theme::change(ThemeMode::Dark, None, cx);
            assert_eq!(Theme::global(cx).theme_name(), "OneNord");
            assert_eq!(Theme::global(cx).background, rgb(0x2E3440).into());

            Theme::change(ThemeMode::Light, None, cx);
            assert_eq!(Theme::global(cx).theme_name(), "OneNord Light");
            assert_eq!(Theme::global(cx).background, rgb(0xF7F8FA).into());

            // The mode switch keeps Jig's fonts.
            if cfg!(target_os = "macos") {
                assert_eq!(
                    Theme::global(cx).mono_font_family,
                    ".AppleSystemUIFontMonospaced"
                );
            }
            assert_eq!(Theme::global(cx).mono_font_size, gpui_kit::px(13.5));
        });
    }

    #[gpui_kit::test]
    fn settings_pick_the_mode_and_font_size(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            init(cx);

            settings::update(cx, |s| {
                s.appearance.theme = ThemeChoice::Dark;
                s.appearance.font_size = 16.;
            });
            apply(None, cx);
            assert_eq!(Theme::global(cx).theme_name(), "OneNord");
            assert_eq!(Theme::global(cx).mono_font_size, gpui_kit::px(16.));

            settings::update(cx, |s| s.appearance.theme = ThemeChoice::Light);
            apply(None, cx);
            assert_eq!(Theme::global(cx).theme_name(), "OneNord Light");
            assert_eq!(
                Theme::global(cx).mono_font_size,
                gpui_kit::px(16.),
                "the size survives a mode switch"
            );
        });
    }

    #[test]
    fn bundled_theme_has_both_modes() {
        let set: ThemeSet = serde_json::from_str(ONENORD).unwrap();
        assert_eq!(set.themes.len(), 2);
        assert!(set.themes.iter().any(|t| t.mode.is_dark()));
        assert!(set.themes.iter().any(|t| !t.mode.is_dark()));
        assert!(set.themes.iter().all(|t| t.highlight.is_some()));
    }
}
