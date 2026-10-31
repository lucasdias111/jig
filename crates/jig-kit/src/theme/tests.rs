use gpui_kit::component::{Theme, ThemeMode, ThemeSet};
use gpui_kit::{TestAppContext, rgb};

use gpui_kit::component::highlighter::HighlightThemeStyle;

use super::{
    EDITABLE, EMBER, SeeThrough, apply, default_color, ember_with, init, parse_hex,
    see_through_editor, to_hex,
};
use jig_settings::{self as settings, ColorSettings, ThemeChoice};

#[gpui_kit::test]
fn ember_applies_in_both_modes(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        init(cx);

        Theme::change(ThemeMode::Dark, None, cx);
        assert_eq!(Theme::global(cx).theme_name(), "Ember");
        assert_eq!(Theme::global(cx).background, rgb(0x272624).into());

        Theme::change(ThemeMode::Light, None, cx);
        assert_eq!(Theme::global(cx).theme_name(), "Ember Light");
        assert_eq!(Theme::global(cx).background, rgb(0xFAF8F5).into());

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
        assert_eq!(Theme::global(cx).theme_name(), "Ember");
        assert_eq!(Theme::global(cx).mono_font_size, gpui_kit::px(16.));

        settings::update(cx, |s| s.appearance.theme = ThemeChoice::Light);
        apply(None, cx);
        assert_eq!(Theme::global(cx).theme_name(), "Ember Light");
        assert_eq!(
            Theme::global(cx).mono_font_size,
            gpui_kit::px(16.),
            "the size survives a mode switch"
        );
    });
}

#[test]
fn see_through_editor_keeps_no_solid_bands() {
    let solid = |hex| parse_hex(hex);
    let mut highlight = HighlightThemeStyle {
        editor_background: solid("#272624"),
        editor_active_line: solid("#2D2C2A"),
        ..Default::default()
    };
    let foreground = solid("#D8D2CA");
    let wrapping = SeeThrough {
        opacity: 0.8,
        wraps: true,
    };
    let mut wrapped = highlight.clone();
    see_through_editor(&mut wrapped, foreground, wrapping);
    assert_eq!(wrapped.editor_background.unwrap().a, 0.8);
    assert_eq!(wrapped.editor_gutter_background.unwrap().a, 0.);
    let line = wrapped.editor_active_line.unwrap();
    assert!(line.a > 0. && line.a < 0.1, "a faint wash, got {}", line.a);

    let scrolling = SeeThrough {
        wraps: false,
        ..wrapping
    };
    see_through_editor(&mut highlight, foreground, scrolling);
    assert_eq!(
        highlight.editor_gutter_background.unwrap().a,
        0.8,
        "covers code scrolled under it"
    );
}

#[test]
fn bundled_theme_has_both_modes() {
    let set: ThemeSet = serde_json::from_str(EMBER).unwrap();
    assert_eq!(set.themes.len(), 2);
    assert!(set.themes.iter().any(|t| t.mode.is_dark()));
    assert!(set.themes.iter().any(|t| !t.mode.is_dark()));
    assert!(set.themes.iter().all(|t| t.highlight.is_some()));
}

#[test]
fn every_editable_color_has_a_bundled_value() {
    for dark in [false, true] {
        for editable in EDITABLE {
            assert!(
                default_color(editable.key, dark).is_some(),
                "{} (dark: {dark})",
                editable.key
            );
        }
    }
}

#[test]
fn hex_round_trips() {
    for hex in ["#F07F9C", "#272624", "#F2935C40"] {
        assert_eq!(to_hex(parse_hex(hex).unwrap()), hex);
    }
    assert!(parse_hex("F07F9C").is_none());
    assert!(parse_hex("#F07").is_none());
    assert!(parse_hex("#GGGGGG").is_none());
}

#[test]
fn user_colors_are_laid_over_the_variant_they_name() {
    let mut colors = ColorSettings::default();
    colors
        .dark
        .insert("syntax.keyword".into(), "#FF0000".into());
    colors
        .dark
        .insert("editor.background".into(), "#000000".into());
    colors
        .dark
        .insert("primary.background".into(), "#00FF00".into());
    colors
        .dark
        .insert("syntax.string".into(), "not a color".into());
    colors.dark.insert("unknown.key".into(), "#123456".into());
    let set = ember_with(&colors);
    let json = serde_json::to_value(&set).unwrap();
    let theme = |dark: bool| {
        json["themes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|t| (t["mode"] == "dark") == dark)
            .unwrap()
            .clone()
    };
    let (dark, light) = (theme(true), theme(false));
    let upper = |v: &serde_json::Value| v.as_str().unwrap().to_uppercase();
    assert!(upper(&dark["highlight"]["syntax"]["keyword"]["color"]).starts_with("#FF0000"));
    assert!(upper(&dark["highlight"]["editor.background"]).starts_with("#000000"));
    assert!(upper(&dark["colors"]["primary.background"]).starts_with("#00FF00"));
    assert!(upper(&dark["highlight"]["syntax"]["string"]["color"]).starts_with("#A9CF6B"));
    assert!(upper(&light["highlight"]["syntax"]["keyword"]["color"]).starts_with("#C2366B"));
}

#[gpui_kit::test]
fn changed_colors_apply_at_once(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        init(cx);
        settings::update(cx, |s| {
            s.appearance.theme = ThemeChoice::Dark;
            s.colors.dark.insert("background".into(), "#101010".into());
        });
    });
    // Observers run once the update's effects are flushed.
    cx.update(|cx| {
        assert_eq!(Theme::global(cx).background, rgb(0x101010).into());
        settings::update(cx, |s| s.colors.dark.clear());
    });
    cx.update(|cx| assert_eq!(Theme::global(cx).background, rgb(0x272624).into()));
}
