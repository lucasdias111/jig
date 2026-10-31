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
    settings.languages.set_off("java", true);
    settings.formatting.set_on_save("rust", true);
    settings.formatting.final_newline = true;
    settings.updates.automatic = false;
    settings.formatting.choose("python", Some("black"));
    settings.agent.using = "codex".into();
    settings
        .agent
        .models
        .insert("claude-code".into(), "opus".into());
    settings.agent.permissions.shell = jig_ai::agent::Access::Allow;
    settings.agent.permissions.questions = false;
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
        AgentSettings::default(),
        "OpenCode, and everything asks"
    );
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
