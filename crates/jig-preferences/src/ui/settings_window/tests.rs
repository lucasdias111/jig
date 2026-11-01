use gpui_kit::component::ActiveTheme as _;
use gpui_kit::{AnyWindowHandle, AppContext as _, Context, TestAppContext, Window};

use super::{
    Detail, Nav, OpenWindow, Page, START, SettingsWindow, appearance_page, init,
    is_settings_window, open,
};
use jig_agent::providers;
use jig_settings::{self as settings, Shortcut, ShortcutGroup, ThemeChoice};

gpui_kit::actions!(settings_tests, [TestSave]);

fn test_save() -> Box<dyn gpui_kit::Action> {
    Box::new(TestSave)
}

/// What the Keyboard page lists in these tests: Save, as the app has it.
const SHORTCUTS: &[Shortcut] = &[Shortcut {
    id: "save",
    label: "Save",
    group: ShortcutGroup::File,
    keys: &["secondary-s"],
    contexts: &[None],
    action: test_save,
}];

fn open_on(nav: Nav, cx: &mut TestAppContext) -> AnyWindowHandle {
    START.set(nav);
    cx.update(|cx| {
        gpui_kit::init(cx);
        jig_kit::theme::init(cx);
        init(cx, SHORTCUTS);
        open(cx);
    });
    cx.run_until_parked();
    START.set(Nav::page(Page::Appearance));
    cx.update(|cx| cx.global::<OpenWindow>().0)
}

fn draw(handle: AnyWindowHandle, cx: &mut TestAppContext) {
    cx.update_window(handle, |_, window, cx| window.draw(cx).clear(cx))
        .unwrap();
}

fn update(
    handle: AnyWindowHandle,
    cx: &mut TestAppContext,
    change: impl FnOnce(&mut SettingsWindow, &mut Window, &mut Context<SettingsWindow>),
) {
    cx.update_window(handle, |root, window, cx| {
        let root = root.downcast::<gpui_kit::component::Root>().unwrap();
        let view = root.read(cx).view().clone().downcast::<SettingsWindow>();
        view.unwrap().update(cx, |this, cx| {
            change(this, window, cx);
            cx.notify();
        });
    })
    .unwrap();
}

#[gpui_kit::test]
fn opens_once_and_reflects_changes(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        jig_kit::theme::init(cx);
        init(cx, SHORTCUTS);
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
    draw(windows[0], cx);
    assert!(cx.update(|cx| cx.theme().is_dark()));
}

#[gpui_kit::test]
fn picking_ember_s_own_color_clears_the_change(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        jig_kit::theme::init(cx);
        settings::update(cx, |s| s.appearance.theme = ThemeChoice::Dark);
    });
    cx.update(|cx| {
        appearance_page::set_color("syntax.keyword", jig_kit::theme::parse_hex("#FF0000"), cx);
        assert_eq!(settings::get(cx).colors.dark["syntax.keyword"], "#FF0000");
        assert!(
            settings::get(cx).colors.light.is_empty(),
            "only the variant in use"
        );

        appearance_page::set_color(
            "syntax.keyword",
            jig_kit::theme::default_color("syntax.keyword", true),
            cx,
        );
        assert!(settings::get(cx).colors.dark.is_empty());
    });
}

#[gpui_kit::test]
fn every_page_draws(cx: &mut TestAppContext) {
    let _dir = providers::init_temp(cx);
    let handle = open_on(Nav::page(Page::Appearance), cx);
    for nav in Nav::everywhere() {
        update(handle, cx, |this, window, cx| this.go(nav, window, cx));
        draw(handle, cx);
    }
}

#[gpui_kit::test]
fn search_finds_rows_on_every_page(cx: &mut TestAppContext) {
    let _dir = providers::init_temp(cx);
    let handle = open_on(Nav::page(Page::Appearance), cx);
    update(handle, cx, |this, window, cx| {
        this.search
            .update(cx, |search, cx| search.set_value("whitespace", window, cx));
    });
    draw(handle, cx);
    update(handle, cx, |this, _, cx| {
        let found: Vec<String> = Nav::everywhere()
            .flat_map(|nav| this.sections(nav, cx))
            .filter_map(|section| section.matching("whitespace"))
            .flat_map(|section| section.rows)
            .map(|row| row.label.to_string())
            .collect();
        assert!(
            found.iter().any(|label| label == "Show whitespace"),
            "{found:?}"
        );
        assert!(
            found
                .iter()
                .any(|label| label == "Trim trailing whitespace"),
            "{found:?}"
        );
    });

    // A language not in the list is found in the Library, by name or
    // extension.
    update(handle, cx, |this, _, cx| {
        let library = this.sections(Nav::detail(Page::Languages, Detail::Library), cx);
        for query in ["ruby", "gemspec"] {
            let found: Vec<String> = library
                .iter()
                .filter_map(|section| section.matching(query))
                .flat_map(|section| section.rows)
                .map(|row| row.id.to_string())
                .collect();
            assert_eq!(found, ["library-ruby"], "{query}");
        }
    });

    // Picking a page leaves the search.
    update(handle, cx, |this, window, cx| {
        this.go(
            Nav::detail(Page::Languages, Detail::Language(0)),
            window,
            cx,
        )
    });
    update(handle, cx, |this, _, cx| assert!(this.query(cx).is_empty()));
    draw(handle, cx);
}

#[gpui_kit::test]
fn the_agent_page_draws_with_every_choice(cx: &mut TestAppContext) {
    let handle = open_on(Nav::page(Page::AgentMode), cx);
    draw(handle, cx);
    for access in jig_ai::agent::Access::ALL {
        cx.update(|cx| {
            settings::update(cx, |s| {
                s.agent.permissions.shell = access;
                s.agent.permissions.questions = !s.agent.permissions.questions;
            })
        });
        draw(handle, cx);
    }
    assert_eq!(
        cx.update(|cx| settings::get(cx).agent.permissions.shell),
        jig_ai::agent::Access::Deny
    );
}

#[gpui_kit::test]
fn the_models_page_draws_in_every_state(cx: &mut TestAppContext) {
    let _dir = providers::init_temp(cx);
    let handle = open_on(Nav::page(Page::Providers), cx);
    draw(handle, cx);

    update(handle, cx, |this, _, _| {
        this.providers_error = Some("Couldn't write config.toml.".into());
    });
    draw(handle, cx);

    // Connected providers: one that takes a key, and a local one.
    cx.update(|cx| {
        let anthropic = jig_ai::template_named("anthropic").unwrap();
        providers::connect(anthropic, Some("sk-test"), cx).unwrap();
        providers::connect(jig_ai::template_named("ollama").unwrap(), None, cx).unwrap();
        providers::set_agent("anthropic/claude-sonnet-5-5", cx).unwrap();
    });
    draw(handle, cx);
    cx.update(|cx| providers::disconnect("ollama", cx).unwrap());
    draw(handle, cx);
}

#[gpui_kit::test]
fn the_updates_page_draws_in_every_state(cx: &mut TestAppContext) {
    use jig_updates::{self as update, State, Version};
    let handle = open_on(Nav::page(Page::Updates), cx);
    let version = Version(9, 0, 0);
    for state in [
        State::Idle,
        State::Checking,
        State::UpToDate,
        State::Downloading(version),
        State::Ready {
            version,
            archive: "Jig.zip".into(),
        },
        State::Manual {
            version,
            reason: "Move Jig to your Applications folder.".into(),
        },
        State::Failed("Couldn't reach GitHub.".into()),
    ] {
        cx.update(|cx| update::set_for_test(state, cx));
        draw(handle, cx);
    }
    cx.update(|cx| settings::update(cx, |s| s.updates.automatic = false));
    draw(handle, cx);
}

#[gpui_kit::test]
fn records_a_shortcut_and_resets_it(cx: &mut TestAppContext) {
    let handle = open_on(Nav::page(Page::Keyboard), cx);
    draw(handle, cx);
    update(handle, cx, |this, window, cx| {
        this.record("save", window, cx)
    });
    cx.simulate_keystrokes(handle, "secondary-shift-u");
    let expected = gpui_kit::Keystroke::parse("secondary-shift-u")
        .unwrap()
        .unparse();
    assert_eq!(
        cx.update(|cx| settings::get(cx).shortcuts.get("save").cloned()),
        Some(expected)
    );
    draw(handle, cx);

    // Pressing the default again forgets the change.
    update(handle, cx, |this, window, cx| {
        this.record("save", window, cx)
    });
    cx.simulate_keystrokes(handle, "secondary-s");
    assert!(cx.update(|cx| settings::get(cx).shortcuts.is_empty()));
}
