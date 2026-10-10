//! Pictures of every Settings page, light and dark, for looking at the
//! design without a screen: `JIG_SNAPSHOTS=<folder> cargo run -p jig-app
//! --features snapshots`. macOS only, since it needs GPUI's Metal renderer,
//! and run from `main` because the Mac platform must be made on the main
//! thread.

use std::path::Path;
use std::sync::Arc;

use gpui_kit::assets::Assets;
use gpui_kit::{
    AppContext as _, Bounds, HeadlessAppContext, WindowBounds, WindowOptions, px, size,
};

use super::{Detail, Nav, Page, START, SettingsWindow};
use crate::settings::{self, ThemeChoice};

pub fn run(dir: &Path) {
    std::fs::create_dir_all(dir).expect("making the snapshots folder");
    let config = std::env::temp_dir().join(format!("jig-snapshots-{}", std::process::id()));
    std::fs::create_dir_all(&config).expect("making a scratch config folder");
    // Each page, then a search.
    let shown = Page::ALL
        .into_iter()
        .map(|page| (Nav::page(page), None))
        .chain([
            (Nav::detail(Page::Appearance, Detail::Colors), None),
            (Nav::detail(Page::Languages, Detail::Language(0)), None),
            (Nav::page(Page::Appearance), Some("format")),
        ]);
    for dark in [false, true] {
        for (ix, (nav, query)) in shown.clone().enumerate() {
            START.set(nav);
            let mut cx = HeadlessAppContext::with_platform(
                gpui_kit::platform::current_platform(true).text_system(),
                Arc::new(Assets),
                gpui_kit::platform::current_headless_renderer,
            );
            cx.update(|cx| {
                gpui_kit::init(cx);
                crate::providers::init_at(&config, cx);
                settings::update(cx, |s| {
                    s.appearance.theme = if dark {
                        ThemeChoice::Dark
                    } else {
                        ThemeChoice::Light
                    }
                });
                crate::theme::init(cx);
                crate::theme::apply(None, cx);
            });
            let (handle, _) = cx
                .update(|cx| {
                    gpui_kit::open_window(
                        WindowOptions {
                            window_bounds: Some(WindowBounds::Windowed(Bounds {
                                origin: Default::default(),
                                size: size(px(820.), px(600.)),
                            })),
                            focus: false,
                            show: false,
                            ..Default::default()
                        },
                        cx,
                        |window, cx| cx.new(|cx| SettingsWindow::new(window, cx)),
                    )
                })
                .expect("opening Settings");
            if let Some(query) = query {
                cx.update_window(handle, |root, window, cx| {
                    let root = root.downcast::<gpui_kit::component::Root>().unwrap();
                    let view = root.read(cx).view().clone().downcast::<SettingsWindow>();
                    view.unwrap().update(cx, |this, cx| {
                        this.search
                            .update(cx, |search, cx| search.set_value(query, window, cx))
                    });
                })
                .expect("searching Settings");
            }
            for _ in 0..3 {
                cx.run_until_parked();
                cx.update_window(handle, |_, window, cx| window.draw(cx).clear(cx))
                    .expect("drawing Settings");
            }
            let image = cx.capture_screenshot(handle).expect("rendering with Metal");
            let name = format!(
                "{ix}-{}-{}.png",
                query.map_or_else(
                    || nav.title().to_lowercase().replace(' ', "-"),
                    |query| format!("search-{query}")
                ),
                if dark { "dark" } else { "light" }
            );
            image.save(dir.join(name)).expect("saving a snapshot");
        }
    }
    std::fs::remove_dir_all(&config).ok();
    START.set(Nav::page(Page::Appearance));
}
