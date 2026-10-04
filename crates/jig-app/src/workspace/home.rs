//! The start pages, shown in place of the editor while no file is open:
//! before a project is open, the home page (start a new file, open a
//! folder, or reopen a recent project); in a project, a few shortcuts.

use std::path::{Path, PathBuf};

use gpui_kit::component::{ActiveTheme as _, Icon, h_flex, v_flex};
use gpui_kit::*;
use jig_editor::EditorHandle;

use super::{FindInFiles, FocusFileTree, GoToFile, NewFile, Workspace};

const FILE_PLUS: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M15 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V7Z"/><path d="M14 2v4a2 2 0 0 0 2 2h4"/><path d="M9 15h6"/><path d="M12 18v-6"/></svg>"#;
const FOLDER_OPEN: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="m6 14 1.5-2.9A2 2 0 0 1 9.24 10H20a2 2 0 0 1 1.94 2.5l-1.54 6a2 2 0 0 1-1.95 1.5H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h3.9a2 2 0 0 1 1.69.9l.81 1.2a2 2 0 0 0 1.67.9H18a2 2 0 0 1 2 2v2"/></svg>"#;
const FOLDER: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="black"><path d="M3 6.5A2.5 2.5 0 0 1 5.5 4h3.88a2 2 0 0 1 1.42.59L12.2 6H18.5A2.5 2.5 0 0 1 21 8.5v9a2.5 2.5 0 0 1-2.5 2.5h-13A2.5 2.5 0 0 1 3 17.5z"/></svg>"#;

const WIDTH: f32 = 420.;

impl Workspace {
    /// Leave the start page for the editor, e.g. once something is opened.
    pub(super) fn leave_home(&mut self, cx: &mut Context<Self>) {
        if self.home {
            self.home = false;
            cx.notify();
        }
    }

    /// Focus what is in the editor's place: the start page or the editor.
    pub(super) fn focus_main(&self, window: &mut Window, cx: &mut Context<Self>) {
        if self.home {
            self.home_focus.focus(window, cx);
        } else {
            self.editor().focus(window, cx);
        }
    }

    /// The start page to show: the home page, or a project's.
    pub(super) fn render_start(&self, cx: &Context<Self>) -> AnyElement {
        match self.project_root(cx) {
            Some(root) => self.render_no_file(&root, cx),
            None => self.render_home(cx),
        }
    }

    /// A project with no file open: its name and the ways to get to one.
    fn render_no_file(&self, root: &Path, cx: &Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let name = root
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        let hint = |label: &'static str, keys: &'static str, action: Box<dyn Action>| {
            h_flex()
                .id(label)
                .h(px(28.))
                .px_2()
                .gap_6()
                .rounded(px(6.))
                .text_size(px(13.))
                .text_color(theme.muted_foreground)
                .cursor_pointer()
                .hover(|s| {
                    s.bg(theme.foreground.opacity(0.06))
                        .text_color(theme.foreground)
                })
                .child(div().flex_1().child(label))
                .child(div().flex_none().child(shortcut_text(keys)))
                .on_click(move |_, window, cx| window.dispatch_action(action.boxed_clone(), cx))
        };
        div()
            .id("no-file")
            .track_focus(&self.home_focus)
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .child(
                v_flex()
                    .w(px(280.))
                    .max_w_full()
                    .px_4()
                    .gap_0p5()
                    .child(
                        div()
                            .px_2()
                            .pb_3()
                            .truncate()
                            .text_size(px(22.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme.foreground.opacity(0.85))
                            .child(name),
                    )
                    .child(hint("Go to File…", "⌘P", Box::new(GoToFile)))
                    .child(hint("Find in Files", "⇧⌘F", Box::new(FindInFiles)))
                    .child(hint("Show Files", "⇧⌘E", Box::new(FocusFileTree)))
                    .child(hint("New File", "⌘N", Box::new(NewFile))),
            )
            .into_any_element()
    }

    fn open_recent(&mut self, project: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        if project.is_dir() {
            self.open_folder(&project, window, cx);
        } else {
            crate::recent::update(cx, |recent| recent.remove(&project));
            self.show_error(
                &format!("{} no longer exists.", display_path(&project)),
                window,
                cx,
            );
            cx.notify();
        }
    }

    fn render_home(&self, cx: &Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let heading = |text: &'static str| {
            div()
                .px_2()
                .pb_1()
                .text_size(px(11.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(theme.muted_foreground)
                .child(text)
        };
        let shortcut = |keys: &'static str| {
            div()
                .flex_none()
                .text_size(px(12.))
                .text_color(theme.muted_foreground)
                .child(shortcut_text(keys))
        };

        let recent = crate::recent::get(cx);
        let recent_rows: Vec<AnyElement> = if recent.is_empty() {
            vec![
                div()
                    .px_2()
                    .py_1()
                    .text_size(px(13.))
                    .text_color(theme.muted_foreground)
                    .child("No recent projects")
                    .into_any_element(),
            ]
        } else {
            recent
                .into_iter()
                .enumerate()
                .map(|(ix, project)| {
                    let name = project
                        .file_name()
                        .map(|name| name.to_string_lossy().into_owned())
                        .unwrap_or_else(|| project.display().to_string());
                    let location = project.parent().map(display_path).unwrap_or_default();
                    row(("recent", ix), FOLDER, cx)
                        .child(div().flex_none().text_color(theme.foreground).child(name))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_size(px(12.))
                                .text_color(theme.muted_foreground)
                                .child(location),
                        )
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.open_recent(project.clone(), window, cx)
                        }))
                        .into_any_element()
                })
                .collect()
        };

        div()
            .id("home")
            .track_focus(&self.home_focus)
            .size_full()
            .flex()
            .justify_center()
            .overflow_y_scroll()
            .child(
                v_flex()
                    .w(px(WIDTH))
                    .max_w_full()
                    .px_4()
                    .pt(relative(0.12))
                    .pb_8()
                    .gap_6()
                    .child(
                        div()
                            .flex()
                            .justify_center()
                            .text_size(px(56.))
                            .line_height(px(60.))
                            .font_weight(FontWeight::BOLD)
                            .text_color(theme.foreground)
                            .child("Jig"),
                    )
                    .child(
                        v_flex()
                            .gap_0p5()
                            .child(heading("Start"))
                            .child(
                                row("home-new-file", FILE_PLUS, cx)
                                    .child(div().flex_1().child("New File"))
                                    .child(shortcut("⌘N"))
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.new_file(&NewFile, window, cx)
                                    })),
                            )
                            .child(
                                row("home-open-folder", FOLDER_OPEN, cx)
                                    .child(div().flex_1().child("Open Folder…"))
                                    .child(shortcut("⌘O"))
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.prompt_open(false, window, cx)
                                    })),
                            ),
                    )
                    .child(
                        v_flex()
                            .gap_0p5()
                            .child(heading("Recent"))
                            .children(recent_rows),
                    ),
            )
            .into_any_element()
    }
}

/// A clickable line on the home page, led by `icon`.
fn row(id: impl Into<ElementId>, icon: &'static [u8], cx: &App) -> Stateful<Div> {
    let theme = cx.theme();
    h_flex()
        .id(id)
        .h(px(30.))
        .px_2()
        .gap_2()
        .rounded(px(6.))
        .text_size(px(13.))
        .text_color(theme.foreground)
        .cursor_pointer()
        .hover(|s| s.bg(theme.foreground.opacity(0.06)))
        .child(
            Icon::default()
                .data(icon)
                .size(px(14.))
                .text_color(theme.muted_foreground),
        )
}

/// `keys` as written on a Mac, spelled out elsewhere.
fn shortcut_text(keys: &str) -> String {
    if cfg!(target_os = "macos") {
        keys.to_string()
    } else {
        keys.replace('⇧', "Shift+").replace('⌘', "Ctrl+")
    }
}

/// `path` with the home folder shortened to `~`.
fn display_path(path: &Path) -> String {
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from)
        && let Ok(rest) = path.strip_prefix(&home)
    {
        return Path::new("~").join(rest).display().to_string();
    }
    path.display().to_string()
}
