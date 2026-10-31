//! The Settings window (⌘,), laid out like a Mac app's: a sidebar of pages
//! (Appearance, Editor, Languages, Keyboard, Updates, and Providers and Agent
//! under AI) with a search field, and the chosen page as titled cards of
//! rows. Languages and theme colors open pages of their own, with a back
//! button in the title bar.

mod agent_page;
mod appearance_page;
mod languages_page;
mod models_page;
mod shortcuts_page;
#[cfg(feature = "snapshots")]
pub mod snapshots;
#[cfg(feature = "snapshots")]
pub use snapshots::run as render_snapshots;
mod ui;
mod updates_page;

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

use jig_agent::providers;
use jig_kit::theme::EDITABLE;
use jig_languages::{self as languages, Language};
use jig_settings::{self as settings, Shortcut};
use ui::Section;

actions!(jig, [OpenSettings]);

// Asked of the editor window from Settings: each opens a file there.
actions!(
    jig,
    [
        EditAgents,
        EditModelConfig,
        EditDebuggers,
        EditFormatters,
        EditLanguageServers
    ]
);

const PALETTE: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M12 22a1 1 0 0 1 0-20 10 9 0 0 1 10 9 5 5 0 0 1-5 5h-2.25a1.75 1.75 0 0 0-1.4 2.8l.3.4a1.75 1.75 0 0 1-1.4 2.8z"/><circle cx="13.5" cy="6.5" r=".5" fill="black"/><circle cx="17.5" cy="10.5" r=".5" fill="black"/><circle cx="6.5" cy="12.5" r=".5" fill="black"/><circle cx="8.5" cy="7.5" r=".5" fill="black"/></svg>"#;
const TEXT_CURSOR: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M17 22h-1a4 4 0 0 1-4-4V6a4 4 0 0 1 4-4h1"/><path d="M7 22h1a4 4 0 0 0 4-4v-1"/><path d="M7 2h1a4 4 0 0 1 4 4v1"/></svg>"#;
const CODE: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="m16 18 6-6-6-6"/><path d="m8 6-6 6 6 6"/></svg>"#;
const KEYBOARD: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect x="2" y="5" width="20" height="14" rx="2"/><path d="M6 9h.01"/><path d="M10 9h.01"/><path d="M14 9h.01"/><path d="M18 9h.01"/><path d="M6 13h.01"/><path d="M18 13h.01"/><path d="M10 13h4"/><path d="M7 16h10"/></svg>"#;
const SPARKLES: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M11.017 2.814a1 1 0 0 1 1.966 0l1.051 5.558a2 2 0 0 0 1.594 1.594l5.558 1.051a1 1 0 0 1 0 1.966l-5.558 1.051a2 2 0 0 0-1.594 1.594l-1.051 5.558a1 1 0 0 1-1.966 0l-1.051-5.558a2 2 0 0 0-1.594-1.594l-5.558-1.051a1 1 0 0 1 0-1.966l5.558-1.051a2 2 0 0 0 1.594-1.594z"/><path d="M20 2v4"/><path d="M22 4h-4"/><circle cx="4" cy="20" r="2"/></svg>"#;
const DOWNLOAD: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="10"/><path d="M12 8v8"/><path d="m8 12 4 4 4-4"/></svg>"#;

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
    Updates,
    Providers,
    AgentMode,
}

impl Page {
    /// In sidebar order; the pages from `Providers` on sit under "AI".
    pub const ALL: [Page; 7] = [
        Page::Appearance,
        Page::Editor,
        Page::Languages,
        Page::Keyboard,
        Page::Updates,
        Page::Providers,
        Page::AgentMode,
    ];

    fn title(self) -> &'static str {
        match self {
            Page::Appearance => "Appearance",
            Page::Editor => "Editor",
            Page::Languages => "Languages",
            Page::Keyboard => "Keyboard",
            Page::Updates => "Updates",
            Page::Providers => "Providers",
            Page::AgentMode => "Agent",
        }
    }

    fn icon(self) -> &'static [u8] {
        match self {
            Page::Appearance => PALETTE,
            Page::Editor => TEXT_CURSOR,
            Page::Languages => CODE,
            Page::Keyboard => KEYBOARD,
            Page::Updates => DOWNLOAD,
            Page::Providers => SPARKLES,
            Page::AgentMode => jig_commands::surface::AGENT_ICON,
        }
    }

    fn is_ai(self) -> bool {
        matches!(self, Page::Providers | Page::AgentMode)
    }
}

/// A page opened from a row of another, with a back button.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Detail {
    Colors,
    /// Every language Jig bundles, to add from.
    Library,
    /// An index into `languages::bundled()`.
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
            Some(Detail::Library) => "Language Library".into(),
            Some(Detail::Language(ix)) => language(ix).label.into(),
        }
    }

    /// Every page and detail page, for search.
    pub fn everywhere() -> impl Iterator<Item = Nav> {
        Page::ALL.into_iter().flat_map(|page| {
            let details: Vec<Detail> = match page {
                Page::Appearance => vec![Detail::Colors],
                Page::Languages => std::iter::once(Detail::Library)
                    .chain((0..languages::bundled().len()).map(Detail::Language))
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
    &languages::bundled()[ix]
}

#[cfg(any(test, feature = "snapshots"))]
thread_local! {
    /// Where a test's Settings window opens.
    static START: std::cell::Cell<Nav> = const { std::cell::Cell::new(Nav::page(Page::Appearance)) };
}

/// The Settings window, while it's open.
struct OpenWindow(AnyWindowHandle);

impl Global for OpenWindow {}

/// The shortcuts the Keyboard page lists, as the app hands them over.
struct ShortcutList(&'static [Shortcut]);

impl Global for ShortcutList {}

fn shortcut_list(cx: &App) -> &'static [Shortcut] {
    cx.try_global::<ShortcutList>().map_or(&[], |list| list.0)
}

/// Bind ⌘, and say which shortcuts the Keyboard page lists: the app's
/// own, which name actions from all over it.
pub fn init(cx: &mut App, shortcuts: &'static [Shortcut]) {
    cx.set_global(ShortcutList(shortcuts));
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
pub fn send_to_workspace(action: Box<dyn Action>, cx: &mut App) {
    jig_commands::windows::send_to_editor(action, cx);
}

/// Work Settings started, such as installing something.
enum Fetch<T> {
    /// What's happening, e.g. "Installing…".
    Running(SharedString),
    Done(T),
    Failed(SharedString),
}

pub struct SettingsWindow {
    nav: Nav,
    search: Entity<InputState>,
    scroll: ScrollHandle,
    /// A masked key field per provider that takes a key, in `TEMPLATES`
    /// order. A key is saved as it's typed and never read back in.
    key_inputs: Vec<(&'static ProviderTemplate, Entity<InputState>)>,
    /// From Install or Uninstall, per debugger or agent key, with what was
    /// done.
    installs: HashMap<String, Fetch<SharedString>>,
    /// Agents asked which models they offer, by key, and how that went.
    model_lookups: HashMap<String, Fetch<SharedString>>,
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
        jig_kit::theme::sync(window, cx);
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Search"));
        let mut subscriptions = vec![
            cx.observe_window_appearance(window, |_, window, cx| jig_kit::theme::sync(window, cx)),
            cx.observe_global::<settings::AppSettings>(|_, cx| cx.notify()),
            cx.observe_global::<providers::Providers>(|_, cx| cx.notify()),
            cx.observe_global::<jig_updates::Updates>(|_, cx| cx.notify()),
            // The providers may have been edited in the meantime.
            cx.observe_window_activation(window, |_, window, cx| {
                if window.is_window_active() {
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
        let this = Self {
            nav,
            search,
            scroll: ScrollHandle::new(),
            key_inputs,
            installs: HashMap::new(),
            model_lookups: HashMap::new(),
            providers_error: None,
            color_pickers,
            recording: None,
            _subscriptions: subscriptions,
        };
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
            (Page::Languages, Some(Detail::Library)) => self.library_sections(cx),
            (Page::Languages, _) => self.languages_sections(cx),
            (Page::Keyboard, _) => self.shortcuts_sections(cx),
            (Page::Updates, _) => self.updates_sections(cx),
            (Page::Providers, _) => self.providers_sections(cx),
            (Page::AgentMode, _) => self.agent_sections(cx),
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
        // A row shown on two pages, like a debugger, is listed once.
        let mut seen = std::collections::HashSet::new();
        let languages = &settings::get(cx).languages;
        let shown = |nav: &Nav| match nav.detail {
            Some(Detail::Language(ix)) => languages.is_added(language(ix).name),
            _ => true,
        };
        for nav in Nav::everywhere().filter(shown) {
            let place = match nav.detail {
                Some(_) => format!("{} › {}", nav.page.title(), nav.title()),
                None => nav.page.title().to_string(),
            };
            for section in self.sections(nav, cx) {
                if let Some(mut section) = section.matching(query) {
                    section.rows.retain(|row| seen.insert(row.id.clone()));
                    if section.rows.is_empty() {
                        continue;
                    }
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
        if self.nav.page == Page::AgentMode {
            self.look_up_agent_models(cx);
        }
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
mod tests;
