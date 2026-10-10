//! The Settings window (⌘,), laid out like a Mac app's: a sidebar of pages
//! (Appearance, Editor, Languages, Keyboard, and Models, Jigs and Agent
//! under AI) with a search field, and the chosen page as titled cards of
//! rows. Languages and theme colors open pages of their own, with a back
//! button in the title bar.

mod agent_page;
mod appearance_page;
mod jigs_page;
mod languages_page;
mod models_page;
mod shortcuts_page;
#[cfg(feature = "snapshots")]
pub mod snapshots;
mod ui;

use std::collections::HashMap;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::color_picker::{ColorPickerEvent, ColorPickerState};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::{
    ActiveTheme as _, Icon, IconName, Sizable as _, TitleBar, h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use jig_ai::{ProviderTemplate, TEMPLATES};

use crate::languages::{self, Language};
use crate::providers;
use crate::settings;
use crate::theme::EDITABLE;
use ui::Section;

actions!(jig, [OpenSettings]);

const PALETTE: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M12 22a1 1 0 0 1 0-20 10 9 0 0 1 10 9 5 5 0 0 1-5 5h-2.25a1.75 1.75 0 0 0-1.4 2.8l.3.4a1.75 1.75 0 0 1-1.4 2.8z"/><circle cx="13.5" cy="6.5" r=".5" fill="black"/><circle cx="17.5" cy="10.5" r=".5" fill="black"/><circle cx="6.5" cy="12.5" r=".5" fill="black"/><circle cx="8.5" cy="7.5" r=".5" fill="black"/></svg>"#;
const TEXT_CURSOR: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M17 22h-1a4 4 0 0 1-4-4V6a4 4 0 0 1 4-4h1"/><path d="M7 22h1a4 4 0 0 0 4-4v-1"/><path d="M7 2h1a4 4 0 0 1 4 4v1"/></svg>"#;
const CODE: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="m16 18 6-6-6-6"/><path d="m8 6-6 6 6 6"/></svg>"#;
const KEYBOARD: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect x="2" y="5" width="20" height="14" rx="2"/><path d="M6 9h.01"/><path d="M10 9h.01"/><path d="M14 9h.01"/><path d="M18 9h.01"/><path d="M6 13h.01"/><path d="M18 13h.01"/><path d="M10 13h4"/><path d="M7 16h10"/></svg>"#;
const SPARKLES: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M11.017 2.814a1 1 0 0 1 1.966 0l1.051 5.558a2 2 0 0 0 1.594 1.594l5.558 1.051a1 1 0 0 1 0 1.966l-5.558 1.051a2 2 0 0 0-1.594 1.594l-1.051 5.558a1 1 0 0 1-1.966 0l-1.051-5.558a2 2 0 0 0-1.594-1.594l-5.558-1.051a1 1 0 0 1 0-1.966l5.558-1.051a2 2 0 0 0 1.594-1.594z"/><path d="M20 2v4"/><path d="M22 4h-4"/><circle cx="4" cy="20" r="2"/></svg>"#;
const COMMAND: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M15 6v12a3 3 0 1 0 3-3H6a3 3 0 1 0 3 3V6a3 3 0 1 0-3 3h12a3 3 0 1 0-3-3"/></svg>"#;

/// The bar across the top, holding the traffic lights and the page's title.
const TITLE_BAR_HEIGHT: f32 = 52.;
const SIDEBAR_WIDTH: f32 = 208.;
/// The widest the cards get, so rows stay readable in a wide window.
const CONTENT_WIDTH: f32 = 620.;

/// A page in the sidebar.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Page {
    Appearance,
    Editor,
    Languages,
    Keyboard,
    Models,
    Jigs,
    Agent,
}

impl Page {
    /// In sidebar order; the pages from `Models` on sit under "AI".
    pub const ALL: [Page; 7] = [
        Page::Appearance,
        Page::Editor,
        Page::Languages,
        Page::Keyboard,
        Page::Models,
        Page::Jigs,
        Page::Agent,
    ];

    fn title(self) -> &'static str {
        match self {
            Page::Appearance => "Appearance",
            Page::Editor => "Editor",
            Page::Languages => "Languages",
            Page::Keyboard => "Keyboard",
            Page::Models => "Models",
            Page::Jigs => "Jigs",
            Page::Agent => "Agent",
        }
    }

    fn icon(self) -> &'static [u8] {
        match self {
            Page::Appearance => PALETTE,
            Page::Editor => TEXT_CURSOR,
            Page::Languages => CODE,
            Page::Keyboard => KEYBOARD,
            Page::Models => SPARKLES,
            Page::Jigs => COMMAND,
            Page::Agent => jig_commands::surface::AGENT_ICON,
        }
    }

    fn is_ai(self) -> bool {
        matches!(self, Page::Models | Page::Jigs | Page::Agent)
    }
}

/// A page opened from a row of another, with a back button.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Detail {
    Colors,
    /// An index into `languages::BUNDLED`.
    Language(usize),
}

/// What the window shows.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Nav {
    pub page: Page,
    pub detail: Option<Detail>,
}

impl Nav {
    pub const fn page(page: Page) -> Self {
        Self { page, detail: None }
    }

    pub const fn detail(page: Page, detail: Detail) -> Self {
        Self {
            page,
            detail: Some(detail),
        }
    }

    fn title(self) -> SharedString {
        match self.detail {
            None => self.page.title().into(),
            Some(Detail::Colors) => "Theme Colors".into(),
            Some(Detail::Language(ix)) => language(ix).label.into(),
        }
    }

    /// Every page and detail page, for search.
    pub fn everywhere() -> impl Iterator<Item = Nav> {
        Page::ALL.into_iter().flat_map(|page| {
            let details: Vec<Detail> = match page {
                Page::Appearance => vec![Detail::Colors],
                Page::Languages => (0..languages::BUNDLED.len())
                    .map(Detail::Language)
                    .collect(),
                _ => Vec::new(),
            };
            std::iter::once(Nav::page(page)).chain(
                details
                    .into_iter()
                    .map(move |detail| Nav::detail(page, detail)),
            )
        })
    }
}

fn language(ix: usize) -> &'static Language {
    &languages::BUNDLED[ix]
}

#[cfg(any(test, feature = "snapshots"))]
thread_local! {
    /// Where a test's Settings window opens.
    static START: std::cell::Cell<Nav> = const { std::cell::Cell::new(Nav::page(Page::Appearance)) };
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
        window_bounds: Some(WindowBounds::centered(size(px(780.), px(600.)), cx)),
        window_min_size: Some(size(px(600.), px(420.))),
        titlebar: Some(TitlebarOptions {
            traffic_light_position: Some(point(px(18.), px(20.))),
            ..TitleBar::title_bar_options()
        }),
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

/// Work Settings started: installing something, or testing a model.
enum Fetch<T> {
    Running,
    Done(T),
    Failed(SharedString),
}

pub struct SettingsWindow {
    nav: Nav,
    search: Entity<InputState>,
    scroll: ScrollHandle,
    /// The user's own jigs, then the built-in ones they haven't replaced.
    user_commands: Vec<jigs_page::CommandRow>,
    built_in_commands: Vec<jigs_page::CommandRow>,
    commands_error: Option<SharedString>,
    /// A masked key field per provider that takes a key, in `TEMPLATES`
    /// order. A key is saved as it's typed and never read back in.
    key_inputs: Vec<(&'static ProviderTemplate, Entity<InputState>)>,
    /// From Install, per debugger or agent key.
    installs: HashMap<String, Fetch<()>>,
    /// From Test: how long the Jig model took to answer.
    check: Option<Fetch<SharedString>>,
    /// Why the last change to the providers didn't work.
    providers_error: Option<SharedString>,
    /// One per entry of `theme::EDITABLE`, in the same order.
    color_pickers: Vec<Entity<ColorPickerState>>,
    /// The shortcut taking new keys, on the Keyboard page.
    recording: Option<shortcuts_page::Recording>,
    _subscriptions: Vec<Subscription>,
}

impl SettingsWindow {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        crate::theme::sync(window, cx);
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Search"));
        let mut subscriptions = vec![
            cx.observe_window_appearance(window, |_, window, cx| crate::theme::sync(window, cx)),
            cx.observe_global::<settings::AppSettings>(|_, cx| cx.notify()),
            cx.observe_global::<providers::Providers>(|_, cx| cx.notify()),
            // Jigs or providers may have been edited in the meantime.
            cx.observe_window_activation(window, |this, window, cx| {
                if window.is_window_active() {
                    this.reload_commands();
                    providers::reload(cx);
                    providers::load_all_models(true, cx);
                    cx.notify();
                }
            }),
            cx.subscribe(&search, |this, _, event, cx| {
                if let InputEvent::Change = event {
                    this.scroll.set_offset(Point::default());
                    cx.notify();
                }
            }),
        ];
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
                            appearance_page::set_color(key, Some(*color), cx);
                        }
                    },
                ));
                picker
            })
            .collect();
        window.set_window_title("Settings");
        #[cfg(any(test, feature = "snapshots"))]
        let nav = START.get();
        #[cfg(not(any(test, feature = "snapshots")))]
        let nav = Nav::page(Page::Appearance);
        let mut this = Self {
            nav,
            search,
            scroll: ScrollHandle::new(),
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

    fn go(&mut self, nav: Nav, window: &mut Window, cx: &mut Context<Self>) {
        if self.nav != nav {
            self.nav = nav;
            self.scroll.set_offset(Point::default());
        }
        // Picking a page leaves the search for it.
        if !self.query(cx).is_empty() {
            self.search
                .update(cx, |search, cx| search.set_value("", window, cx));
        }
        cx.notify();
    }

    /// A row's click handler that shows `nav`.
    fn opener(&self, nav: Nav, cx: &Context<Self>) -> impl Fn(&mut Window, &mut App) + 'static {
        let this = cx.entity().downgrade();
        move |window, cx| {
            this.update(cx, |this, cx| this.go(nav, window, cx)).ok();
        }
    }

    fn query(&self, cx: &App) -> String {
        self.search.read(cx).value().trim().to_string()
    }

    fn sections(&self, nav: Nav, cx: &Context<Self>) -> Vec<Section> {
        match (nav.page, nav.detail) {
            (Page::Appearance, Some(Detail::Colors)) => self.colors_sections(cx),
            (Page::Appearance, _) => self.appearance_sections(cx),
            (Page::Editor, _) => appearance_page::editor_sections(),
            (Page::Languages, Some(Detail::Language(ix))) => self.language_sections(ix, cx),
            (Page::Languages, _) => self.languages_sections(cx),
            (Page::Keyboard, _) => self.shortcuts_sections(cx),
            (Page::Models, _) => self.models_sections(cx),
            (Page::Jigs, _) => self.jigs_sections(),
            (Page::Agent, _) => self.agent_sections(cx),
        }
    }

    fn render_sidebar(&self, cx: &Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let searching = !self.query(cx).is_empty();
        let item =
            |page: Page| {
                let selected = !searching && self.nav.page == page;
                let this = cx.entity().downgrade();
                let hover = theme.sidebar_accent.opacity(0.6);
                h_flex()
                    .id(SharedString::from(format!("page-{}", page.title())))
                    .h(px(28.))
                    .px_2()
                    .gap_2()
                    .rounded(px(6.))
                    .text_size(px(ui::TEXT))
                    .when(selected, |this| {
                        this.bg(theme.sidebar_accent)
                            .text_color(theme.sidebar_accent_foreground)
                    })
                    .when(!selected, |this| this.hover(move |style| style.bg(hover)))
                    .child(Icon::default().data(page.icon()).size(px(15.)).text_color(
                        if selected {
                            theme.sidebar_accent_foreground
                        } else {
                            theme.muted_foreground
                        },
                    ))
                    .child(page.title())
                    .on_click(move |_, window, cx| {
                        this.update(cx, |this, cx| this.go(Nav::page(page), window, cx))
                            .ok();
                    })
            };
        v_flex()
            .w(px(SIDEBAR_WIDTH))
            .h_full()
            .flex_shrink_0()
            .bg(theme.sidebar)
            .text_color(theme.sidebar_foreground)
            .border_r_1()
            .border_color(ui::hairline(cx))
            .pt(px(TITLE_BAR_HEIGHT))
            .px_2p5()
            .gap_0p5()
            .child(
                div().pb_3().child(
                    Input::new(&self.search).small().cleanable(true).prefix(
                        Icon::new(IconName::Search)
                            .size(px(14.))
                            .text_color(theme.muted_foreground),
                    ),
                ),
            )
            .children(
                Page::ALL
                    .into_iter()
                    .filter(|page| !page.is_ai())
                    .map(&item),
            )
            .child(
                div()
                    .pt_4()
                    .pb_1()
                    .px_2()
                    .text_size(px(11.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme.muted_foreground)
                    .child("AI"),
            )
            .children(Page::ALL.into_iter().filter(|page| page.is_ai()).map(item))
    }

    /// Every row matching the search, under the page and section it's in.
    fn search_results(&self, query: &str, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let mut found = Vec::new();
        for nav in Nav::everywhere() {
            let place = match nav.detail {
                Some(_) => format!("{} › {}", nav.page.title(), nav.title()),
                None => nav.page.title().to_string(),
            };
            for section in self.sections(nav, cx) {
                if let Some(mut section) = section.matching(query) {
                    section.title = Some(match &section.title {
                        Some(title) => format!("{place} › {title}").into(),
                        None => place.clone().into(),
                    });
                    section.accessory = None;
                    found.push(section);
                }
            }
        }
        if found.is_empty() {
            return v_flex().pt_16().items_center().child(
                div()
                    .text_size(px(ui::TEXT))
                    .text_color(cx.theme().muted_foreground)
                    .child(format!("No settings match “{query}”.")),
            );
        }
        ui::render_sections("search", &found, window, cx)
    }

    fn render_title(&self, cx: &Context<Self>) -> impl IntoElement {
        let searching = !self.query(cx).is_empty();
        let title: SharedString = if searching {
            "Search Results".into()
        } else {
            self.nav.title()
        };
        let back = (!searching && self.nav.detail.is_some()).then(|| {
            let this = cx.entity().downgrade();
            let page = self.nav.page;
            Button::new("back")
                .icon(IconName::ChevronLeft)
                .ghost()
                .small()
                .tooltip(format!("Back to {}", page.title()))
                .on_click(move |_, window, cx| {
                    this.update(cx, |this, cx| this.go(Nav::page(page), window, cx))
                        .ok();
                })
        });
        h_flex()
            .size_full()
            .pl(px(SIDEBAR_WIDTH + 16.))
            .gap_1()
            .items_center()
            .children(back)
            .child(
                div()
                    .text_size(px(15.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(title),
            )
    }

    /// Show a failed change to the providers until the next one works.
    fn report(&mut self, result: Result<(), String>, cx: &mut Context<Self>) {
        self.providers_error = result.err().map(SharedString::from);
        cx.notify();
    }
}

impl Render for SettingsWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.sync_color_pickers(window, cx);
        let query = self.query(cx);
        let content = if query.is_empty() {
            let sections = self.sections(self.nav, cx);
            ui::render_sections(&format!("{:?}", self.nav), &sections, window, cx)
        } else {
            self.search_results(&query, window, cx)
        };
        let theme = cx.theme();
        div()
            .size_full()
            .relative()
            .bg(theme.background)
            .text_color(theme.foreground)
            .child(
                h_flex().size_full().child(self.render_sidebar(cx)).child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .h_full()
                        .child(
                            div()
                                .h(px(TITLE_BAR_HEIGHT))
                                .flex_shrink_0()
                                .border_b_1()
                                .border_color(ui::hairline(cx)),
                        )
                        .child(
                            div()
                                .relative()
                                .flex_1()
                                .min_h_0()
                                .child(
                                    div()
                                        .id("settings-page")
                                        .size_full()
                                        .overflow_y_scroll()
                                        .track_scroll(&self.scroll)
                                        .child(
                                            h_flex().w_full().justify_center().child(
                                                div()
                                                    .w_full()
                                                    .max_w(px(CONTENT_WIDTH))
                                                    .px_6()
                                                    .pt_5()
                                                    .pb_10()
                                                    .child(content),
                                            ),
                                        ),
                                )
                                .vertical_scrollbar(&self.scroll),
                        ),
                ),
            )
            .child(
                // Over both columns, so the window drags from anywhere
                // along the top, as on the Mac.
                div().absolute().top_0().left_0().right_0().child(
                    TitleBar::new()
                        .h(px(TITLE_BAR_HEIGHT))
                        .pl_0()
                        .border_b_0()
                        .bg(transparent_black())
                        .child(self.render_title(cx)),
                ),
            )
    }
}

#[cfg(test)]
mod tests {
    use gpui_kit::component::ActiveTheme as _;
    use gpui_kit::{AnyWindowHandle, AppContext as _, Context, TestAppContext, Window};

    use super::{
        Detail, Fetch, Nav, OpenWindow, Page, START, SettingsWindow, appearance_page, init,
        is_settings_window, open,
    };
    use crate::providers;
    use crate::settings::{self, ThemeChoice};

    fn open_on(nav: Nav, cx: &mut TestAppContext) -> AnyWindowHandle {
        START.set(nav);
        cx.update(|cx| {
            gpui_kit::init(cx);
            crate::theme::init(cx);
            init(cx);
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
        draw(windows[0], cx);
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
            appearance_page::set_color("syntax.keyword", crate::theme::parse_hex("#FF0000"), cx);
            assert_eq!(settings::get(cx).colors.dark["syntax.keyword"], "#FF0000");
            assert!(
                settings::get(cx).colors.light.is_empty(),
                "only the variant in use"
            );

            appearance_page::set_color(
                "syntax.keyword",
                crate::theme::default_color("syntax.keyword", true),
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
        let handle = open_on(Nav::page(Page::Agent), cx);
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
        let handle = open_on(Nav::page(Page::Models), cx);
        draw(handle, cx);

        update(handle, cx, |this, _, _| {
            this.check = Some(Fetch::Failed("401 Unauthorized".into()));
            this.providers_error = Some("Couldn't write config.toml.".into());
        });
        draw(handle, cx);

        // Connected providers: one that takes a key, and a local one.
        cx.update(|cx| {
            let anthropic = jig_ai::template_named("anthropic").unwrap();
            providers::connect(anthropic, Some("sk-test"), cx).unwrap();
            providers::connect(jig_ai::template_named("ollama").unwrap(), None, cx).unwrap();
            providers::set_quick("anthropic/claude-haiku-4-5", cx).unwrap();
            providers::set_agent(Some("anthropic/claude-sonnet-5-5"), cx).unwrap();
        });
        draw(handle, cx);
        cx.update(|cx| providers::disconnect("opencode-go", cx).unwrap());
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
}
