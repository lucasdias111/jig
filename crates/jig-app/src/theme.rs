//! Jig's look: OneNord, in a light and a dark variant that follow the system.

use std::rc::Rc;

use gpui_kit::component::{Theme, ThemeSet};
use gpui_kit::{App, Window};

const ONENORD: &str = include_str!("../../../assets/themes/onenord.json");

/// Make OneNord the light and dark theme. Call once, after `gpui_kit::init`.
pub fn init(cx: &mut App) {
    let set: ThemeSet = serde_json::from_str(ONENORD).expect("bundled onenord.json is valid");
    let mut light = None;
    let mut dark = None;
    for theme in set.themes {
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
}

/// Apply the variant matching the window's current appearance.
pub fn sync(window: &mut Window, cx: &mut App) {
    Theme::sync_system_appearance(Some(window), cx);
}

#[cfg(test)]
mod tests {
    use gpui_kit::component::{Theme, ThemeMode, ThemeSet};
    use gpui_kit::{TestAppContext, rgb};

    use super::{ONENORD, init};

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
