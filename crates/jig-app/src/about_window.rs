//! About Jig: a small window with the app's icon, its name and version, as
//! a Mac app's About panel shows them, and a way to check for a newer one.

use std::sync::{Arc, LazyLock};

use gpui_kit::component::button::Button;
use gpui_kit::component::{ActiveTheme as _, Sizable as _, TitleBar, v_flex};
use gpui_kit::*;

use jig_updates as update;

const ICON_PNG: &[u8] = include_bytes!("../../../assets/app-icon/jig-1024.png");

/// Decoded once, the first time the window opens.
static ICON: LazyLock<Arc<Image>> =
    LazyLock::new(|| Arc::new(Image::from_bytes(ImageFormat::Png, ICON_PNG.to_vec())));

const WIDTH: f32 = 280.;
const HEIGHT: f32 = 300.;

/// The About window, while it's open.
struct OpenWindow(AnyWindowHandle);

impl Global for OpenWindow {}

/// Show the About window, opening it if needed.
pub fn open(cx: &mut App) {
    if let Some(OpenWindow(handle)) = cx.try_global::<OpenWindow>()
        && handle
            .update(cx, |_, window, _| window.activate_window())
            .is_ok()
    {
        return;
    }
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::centered(size(px(WIDTH), px(HEIGHT)), cx)),
        titlebar: Some(TitlebarOptions {
            traffic_light_position: Some(point(px(12.), px(12.))),
            ..TitleBar::title_bar_options()
        }),
        is_resizable: false,
        is_minimizable: false,
        ..TitleBar::window_options()
    };
    match gpui_kit::open_window(options, cx, |window, cx| {
        cx.new(|cx| AboutWindow::new(window, cx))
    }) {
        Ok((handle, _)) => cx.set_global(OpenWindow(handle)),
        Err(error) => eprintln!("jig: couldn't open About Jig: {error:#}"),
    }
    cx.activate(true);
}

/// Whether `window` is the About window.
pub fn is_about_window(window: AnyWindowHandle, cx: &App) -> bool {
    cx.try_global::<OpenWindow>()
        .is_some_and(|OpenWindow(handle)| *handle == window)
}

pub(crate) struct AboutWindow {
    _appearance: Subscription,
}

impl AboutWindow {
    pub(crate) fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        crate::theme::sync(window, cx);
        window.set_window_title("About Jig");
        Self {
            _appearance: cx
                .observe_window_appearance(window, |_, window, cx| crate::theme::sync(window, cx)),
        }
    }
}

impl Render for AboutWindow {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        div()
            .size_full()
            .relative()
            .bg(theme.background)
            .text_color(theme.foreground)
            .child(
                v_flex()
                    .size_full()
                    .items_center()
                    .justify_center()
                    .pt_4()
                    .child(img(ICON.clone()).size(px(96.)))
                    .child(
                        div()
                            .pt_2()
                            .text_size(px(17.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Jig"),
                    )
                    .child(
                        div()
                            .pt_0p5()
                            .text_size(px(12.))
                            .text_color(theme.muted_foreground)
                            .child(format!("Version {}", update::current_version())),
                    )
                    .child(
                        div()
                            .pt_3()
                            .text_size(px(12.))
                            .text_color(theme.muted_foreground)
                            .child("A coding agent at the cursor."),
                    )
                    .child(
                        div().pt_5().child(
                            Button::new("check-for-updates")
                                .label("Check for Updates…")
                                .small()
                                .outline()
                                .on_click(|_, _, cx| update::check(true, cx)),
                        ),
                    ),
            )
            .child(
                // Drags the window from its top, where the title bar would be.
                div().absolute().top_0().left_0().right_0().child(
                    TitleBar::new()
                        .h(px(36.))
                        .border_b_0()
                        .bg(transparent_black()),
                ),
            )
    }
}

#[cfg(test)]
mod tests {
    use gpui_kit::{AppContext as _, TestAppContext};

    use super::{is_about_window, open};

    #[gpui_kit::test]
    fn opens_once_and_draws(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            crate::theme::init(cx);
            open(cx);
            open(cx);
        });
        cx.run_until_parked();
        let windows = cx.update(|cx| cx.windows());
        assert_eq!(windows.len(), 1, "a second About reuses the window");
        assert!(cx.update(|cx| is_about_window(windows[0], cx)));
        cx.update_window(windows[0], |_, window, cx| window.draw(cx).clear(cx))
            .unwrap();
    }
}
