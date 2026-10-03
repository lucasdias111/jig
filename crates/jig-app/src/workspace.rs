//! The one window: a single document in a single editor.

mod commands;
mod user_commands;

use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::component::input::{Editor, EditorState, InputEvent};
use gpui_kit::component::{ActiveTheme as _, TitleBar, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use jig_ai::Provider;
use jig_commands::{CommandPalette, Preset, presets};
use jig_editor::{EditorHandle, KitEditor};

use crate::document::Document;
use commands::CommandRun;

actions!(
    jig,
    [
        Quit,
        Open,
        Save,
        SaveAs,
        CloseWindow,
        OpenCommand,
        AddCommand,
        EditCommands,
        EditProjectRules
    ]
);

pub struct Workspace {
    document: Document,
    editor: KitEditor,
    dirty: bool,
    presets: Rc<Vec<Preset>>,
    palette: Option<OpenPalette>,
    new_command: Option<user_commands::OpenForm>,
    /// The user's commands file, `~/.config/jig/commands.toml`.
    commands_path: Option<PathBuf>,
    /// The configured model, or why it couldn't be set up.
    provider: Result<Arc<dyn Provider>, String>,
    run: Option<CommandRun>,
    next_run_id: u64,
    _editor_events: Subscription,
}

struct OpenPalette {
    view: Entity<CommandPalette>,
    /// Window position of the palette's top-left corner, fixed when it opens.
    anchor: Point<Pixels>,
    _events: Subscription,
}

impl Workspace {
    pub fn new(path: Option<PathBuf>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let (document, error) = match path.as_deref().map(Document::open) {
            Some(Ok(document)) => (document, None),
            Some(Err(error)) => (Document::default(), Some(error)),
            None => (Document::default(), None),
        };
        let (editor, subscription) = Self::build_editor(&document, window, cx);
        let (presets, presets_error) = match presets::load(presets::user_commands_path().as_deref())
        {
            Ok(presets) => (presets, None),
            Err(error) => (presets::defaults(), Some(error)),
        };
        let mut this = Self {
            document,
            editor,
            dirty: false,
            presets: Rc::new(presets),
            palette: None,
            new_command: None,
            commands_path: presets::user_commands_path(),
            provider: commands::load_provider(),
            run: None,
            next_run_id: 0,
            _editor_events: subscription,
        };
        this.update_title(window);
        crate::theme::sync(window, cx);
        cx.observe_window_appearance(window, |_, window, cx| crate::theme::sync(window, cx))
            .detach();
        if let Some(error) = presets_error {
            this.show_error(
                &format!("Using the built-in commands. {error:#}"),
                window,
                cx,
            );
        }

        window.on_window_should_close(cx, {
            let workspace = cx.entity().downgrade();
            move |window, cx| {
                workspace
                    .update(cx, |this, cx| this.confirm_close(window, cx))
                    .unwrap_or(true)
            }
        });

        if let Some(error) = error {
            this.show_error(&format!("{error:#}"), window, cx);
        } else if path.is_none() {
            this.prompt_open(window, cx);
        }
        this
    }

    fn build_editor(
        document: &Document,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> (KitEditor, Subscription) {
        let state = cx.new(|cx| {
            EditorState::new(window, cx)
                .language(document.language())
                .default_value(document.saved_text.clone())
        });
        let subscription =
            cx.subscribe_in(&state, window, |this, _, event: &InputEvent, window, cx| {
                if matches!(event, InputEvent::Change) {
                    this.refresh_dirty(window, cx);
                    this.on_buffer_changed(cx);
                }
                cx.notify();
            });
        let editor = KitEditor::new(state, cx);
        editor.focus(window, cx);
        (editor, subscription)
    }

    fn load(&mut self, path: &Path, window: &mut Window, cx: &mut Context<Self>) {
        match Document::open(path) {
            Ok(document) => {
                let (editor, subscription) = Self::build_editor(&document, window, cx);
                self.document = document;
                self.editor = editor;
                self._editor_events = subscription;
                self.dirty = false;
                self.update_title(window);
                cx.notify();
            }
            Err(error) => self.show_error(&format!("{error:#}"), window, cx),
        }
    }

    fn refresh_dirty(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let dirty = self.document.is_dirty(&self.editor.text(cx));
        if dirty != self.dirty {
            self.dirty = dirty;
            self.update_title(window);
        }
    }

    fn update_title(&self, window: &mut Window) {
        let title = self.document.title();
        let marker = if self.dirty { " •" } else { "" };
        window.set_window_title(&format!("{title}{marker}"));
        window.set_window_edited(self.dirty);
        window.set_document_path(self.document.path.as_deref());
    }

    fn show_error(&self, message: &str, window: &mut Window, cx: &mut Context<Self>) {
        // The only answer is OK, so nothing waits on it.
        drop(window.prompt(PromptLevel::Critical, "Jig", Some(message), &["OK"], cx));
    }

    /// Run `then` now if there are no unsaved changes, otherwise only after
    /// the user agrees to discard them.
    fn when_discard_ok(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        then: impl FnOnce(&mut Self, &mut Window, &mut Context<Self>) + 'static,
    ) {
        if !self.dirty {
            then(self, window, cx);
            return;
        }
        let detail = format!("{} has unsaved changes.", self.document.title());
        let answer = window.prompt(
            PromptLevel::Warning,
            "Discard unsaved changes?",
            Some(&detail),
            &["Discard", "Cancel"],
            cx,
        );
        cx.spawn_in(window, async move |this, cx| {
            if answer.await == Ok(0) {
                this.update_in(cx, |this, window, cx| then(this, window, cx))
                    .ok();
            }
        })
        .detach();
    }

    fn confirm_close(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if !self.dirty {
            return true;
        }
        self.when_discard_ok(window, cx, |this, window, _| {
            this.dirty = false;
            window.remove_window();
        });
        false
    }

    fn prompt_open(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: None,
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = paths.await else {
                return;
            };
            let Some(path) = paths.into_iter().next() else {
                return;
            };
            this.update_in(cx, |this, window, cx| this.load(&path, window, cx))
                .ok();
        })
        .detach();
    }

    fn open(&mut self, _: &Open, window: &mut Window, cx: &mut Context<Self>) {
        self.when_discard_ok(window, cx, |this, window, cx| this.prompt_open(window, cx));
    }

    fn save(&mut self, _: &Save, window: &mut Window, cx: &mut Context<Self>) {
        match self.document.path.clone() {
            Some(path) => self.save_to(&path, window, cx),
            None => self.prompt_save_as(window, cx),
        }
    }

    fn save_as(&mut self, _: &SaveAs, window: &mut Window, cx: &mut Context<Self>) {
        self.prompt_save_as(window, cx);
    }

    fn prompt_save_as(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let dir = self
            .document
            .path
            .as_deref()
            .and_then(Path::parent)
            .map(Path::to_path_buf)
            .or_else(|| std::env::current_dir().ok())
            .unwrap_or_default();
        let name = self.document.title();
        let path = cx.prompt_for_new_path(&dir, Some(&name));
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(path))) = path.await else {
                return;
            };
            this.update_in(cx, |this, window, cx| this.save_to(&path, window, cx))
                .ok();
        })
        .detach();
    }

    fn save_to(&mut self, path: &Path, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.editor.text(cx);
        let language_changed = crate::document::language_for(path) != self.document.language();
        if let Err(error) = self.document.save(path, &text) {
            self.show_error(&format!("Could not save: {error:#}"), window, cx);
            return;
        }
        if language_changed {
            // Rebuild so highlighting matches the new extension.
            self.load(path, window, cx);
        }
        self.dirty = false;
        self.update_title(window);
        if self.is_commands_file() {
            self.reload_presets(window, cx);
        }
        cx.notify();
    }

    fn close_window(&mut self, _: &CloseWindow, window: &mut Window, cx: &mut Context<Self>) {
        self.when_discard_ok(window, cx, |this, window, _| {
            this.dirty = false;
            window.remove_window();
        });
    }

    fn quit(&mut self, _: &Quit, window: &mut Window, cx: &mut Context<Self>) {
        self.when_discard_ok(window, cx, |_, _, cx| cx.quit());
    }
}

pub fn key_bindings() -> Vec<KeyBinding> {
    vec![
        KeyBinding::new("secondary-q", Quit, None),
        KeyBinding::new("secondary-o", Open, None),
        KeyBinding::new("secondary-s", Save, None),
        KeyBinding::new("secondary-shift-s", SaveAs, None),
        KeyBinding::new("secondary-w", CloseWindow, None),
        KeyBinding::new("secondary-k", OpenCommand, None),
        KeyBinding::new("secondary-shift-k", AddCommand, None),
    ]
    .into_iter()
    .chain(jig_commands::new_command::key_bindings())
    .chain(jig_commands::palette::key_bindings())
    .collect()
}

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let viewport = window.viewport_size();
        let theme = cx.theme();
        let title = self.document.title();
        v_flex()
            .size_full()
            .bg(theme.background)
            .on_action(cx.listener(Self::quit))
            .on_action(cx.listener(Self::open))
            .on_action(cx.listener(Self::save))
            .on_action(cx.listener(Self::save_as))
            .on_action(cx.listener(Self::close_window))
            .on_action(cx.listener(Self::open_command))
            .on_action(cx.listener(Self::add_command))
            .on_action(cx.listener(Self::edit_commands))
            .on_action(cx.listener(Self::edit_project_rules))
            .capture_action(cx.listener(Self::on_escape))
            .capture_action(cx.listener(Self::on_accept_enter))
            .capture_action(cx.listener(Self::on_accept_tab))
            .capture_action(cx.listener(Self::on_accept_indent))
            .capture_action(cx.listener(Self::on_undo))
            .child(
                TitleBar::new().child(
                    h_flex()
                        .flex_1()
                        .justify_center()
                        // Balance the traffic lights so the title sits centred.
                        .mr(px(70.))
                        .gap_1p5()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child(title)
                        .when(self.dirty, |this| {
                            this.child(div().size(px(6.)).rounded_full().bg(theme.muted_foreground))
                        }),
                ),
            )
            .child(
                div().flex_1().min_h_0().pl_2().pr_3().pb_2().child(
                    Editor::new(self.editor.state())
                        .bordered(false)
                        // Locked while a command's change awaits review. The
                        // element re-applies this every frame.
                        .readonly(self.previewing())
                        .size_full(),
                ),
            )
            .when_some(self.new_command.as_ref(), |this, form| {
                // Centred near the top, like a sheet.
                let left = ((viewport.width - px(460.)) / 2.).max(px(8.));
                this.child(deferred(
                    anchored()
                        .position(gpui_kit::point(left, px(56.)))
                        .snap_to_window_with_margin(px(8.))
                        .child(form.view.clone()),
                ))
            })
            .when_some(self.palette.as_ref(), |this, palette| {
                this.child(deferred(
                    anchored()
                        .position(palette.anchor)
                        .snap_to_window_with_margin(px(8.))
                        .child(palette.view.clone()),
                ))
            })
            .when_some(self.run.as_ref(), |this, run| {
                this.child(deferred(
                    anchored()
                        .position(run.anchor)
                        .snap_to_window_with_margin(px(8.))
                        .child(run.bubble.clone()),
                ))
            })
    }
}

#[cfg(test)]
mod tests {
    use gpui_kit::test::TestWindowExt;
    use std::path::Path;

    use gpui_kit::{
        AnyWindowHandle, AppContext, Bounds, Entity, Focusable, Point, TestAppContext,
        WindowBounds, WindowOptions, px, size,
    };
    use std::sync::Arc;

    use jig_ai::Provider;
    use jig_commands::Bubble;
    use jig_editor::EditorHandle;

    use super::{Workspace, key_bindings};

    /// Answers every request with a fixed reply.
    struct FakeProvider(Result<&'static str, &'static str>);

    impl Provider for FakeProvider {
        fn complete(&self, _: &str, user: &str) -> anyhow::Result<String> {
            assert!(user.contains("<<<SELECTION>>>"), "the target is marked");
            self.0.map(str::to_string).map_err(|e| anyhow::anyhow!(e))
        }
    }

    fn use_provider(
        cx: &mut TestAppContext,
        workspace: &Entity<Workspace>,
        reply: Result<&'static str, &'static str>,
    ) {
        cx.update(|cx| {
            workspace.update(cx, |this, _| {
                this.provider = Ok(Arc::new(FakeProvider(reply)))
            })
        });
    }

    /// Select `range`, then run the preset matching `query` through Cmd+K.
    fn run_preset(
        cx: &mut TestAppContext,
        window: AnyWindowHandle,
        workspace: &Entity<Workspace>,
        range: std::ops::Range<usize>,
        query: &str,
    ) {
        let workspace = workspace.clone();
        let query = query.to_string();
        step(cx, window, move |window, cx| {
            window.render_frame(cx);
            workspace.update(cx, |this, cx| {
                this.editor
                    .state()
                    .update(cx, |s, cx| s.set_selected_range(range, cx))
            });
            window.press("secondary-k", cx);
        });
        step(cx, window, move |window, cx| {
            window.render_frame(cx);
            window.input(&query, cx);
            window.press("enter", cx);
        });
    }

    fn bubble(cx: &mut TestAppContext, workspace: &Entity<Workspace>) -> Option<Bubble> {
        cx.update(|cx| {
            workspace
                .read(cx)
                .run
                .as_ref()
                .map(|run| run.bubble.clone())
        })
    }

    fn open(cx: &mut TestAppContext, path: &Path) -> (AnyWindowHandle, Entity<Workspace>) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            cx.bind_keys(key_bindings());
        });
        let path = path.to_path_buf();
        let handles = cx.update(|cx| {
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds {
                    origin: Point::default(),
                    size: size(px(800.), px(480.)),
                })),
                ..Default::default()
            };
            gpui_kit::open_window(options, cx, |window, cx| {
                cx.new(|cx| Workspace::new(Some(path), window, cx))
            })
            .unwrap()
        });
        cx.run_until_parked();
        handles
    }

    /// Run `f` in the window, then let subscriptions and effects settle.
    fn step(
        cx: &mut TestAppContext,
        window: AnyWindowHandle,
        f: impl FnOnce(&mut gpui_kit::Window, &mut gpui_kit::App),
    ) {
        cx.update_window(window, |_, window, cx| f(window, cx))
            .unwrap();
        cx.run_until_parked();
    }

    #[gpui_kit::test]
    fn edit_marks_dirty_and_save_writes_file(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("lib.rs");
        std::fs::write(&path, "fn a() {}\n").unwrap();
        let (window, workspace) = open(cx, &path);

        step(cx, window, |window, cx| {
            window.render_frame(cx);
            assert_eq!(workspace.read(cx).editor.language(cx), "rust");
            assert!(!workspace.read(cx).dirty);
            window.press("secondary-down", cx);
            window.input("// x\n", cx);
        });
        step(cx, window, |window, cx| {
            assert!(workspace.read(cx).dirty, "typing marks the buffer dirty");
            window.press("secondary-s", cx);
        });
        step(cx, window, |_, cx| {
            assert!(!workspace.read(cx).dirty, "saving clears the dirty flag")
        });
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "fn a() {}\n// x\n");
    }

    #[gpui_kit::test]
    fn undoing_back_to_saved_text_clears_dirty(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("lib.rs");
        std::fs::write(&path, "fn a() {}\n").unwrap();
        let (window, workspace) = open(cx, &path);

        step(cx, window, |window, cx| {
            window.render_frame(cx);
            window.input("x", cx);
        });
        step(cx, window, |window, cx| {
            assert!(workspace.read(cx).dirty);
            window.press("secondary-z", cx);
        });
        step(cx, window, |_, cx| assert!(!workspace.read(cx).dirty));
    }

    const DOCS_REPLY: &str =
        r#"{"replace": "/// Does a.\nfn a() {}", "message": "Added a doc comment."}"#;
    const ORIGINAL: &str = "fn a() {}\n";
    const DOCUMENTED: &str = "/// Does a.\nfn a() {}\n";

    /// Open `lib.rs`, run "Add docs" on `fn a() {}` and wait for the preview.
    fn preview_docs(
        cx: &mut TestAppContext,
    ) -> (tempfile::TempDir, AnyWindowHandle, Entity<Workspace>) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("lib.rs");
        std::fs::write(&path, ORIGINAL).unwrap();
        let (window, workspace) = open(cx, &path);
        use_provider(cx, &workspace, Ok(DOCS_REPLY));
        run_preset(cx, window, &workspace, 0..9, "docs");
        (dir, window, workspace)
    }

    fn text(cx: &mut TestAppContext, workspace: &Entity<Workspace>) -> String {
        cx.update(|cx| workspace.read(cx).editor.text(cx))
    }

    #[gpui_kit::test]
    fn reply_is_previewed_in_place(cx: &mut TestAppContext) {
        let (_dir, window, workspace) = preview_docs(cx);
        step(cx, window, |window, cx| {
            let this = workspace.read(cx);
            assert!(
                this.palette.is_none(),
                "running a command closes the palette"
            );
            assert!(
                this.editor.state().focus_handle(cx).is_focused(window),
                "focus returns to the editor"
            );
            let run = this.run.as_ref().expect("a command ran");
            assert_eq!(run.target, 0..9);
            assert_eq!(
                run.preview.as_ref().unwrap().range,
                0..21,
                "the new code's range"
            );
            assert_eq!(
                run.bubble,
                Bubble::Preview {
                    message: "Added a doc comment.".into(),
                    removed: "fn a() {}".into()
                }
            );
            assert_eq!(
                this.editor.text(cx),
                DOCUMENTED,
                "the change is in the buffer"
            );
            assert!(
                !this.editor.state().read(cx).is_editable(),
                "and locked while under review"
            );
        });
    }

    #[gpui_kit::test]
    fn enter_accepts_and_one_undo_reverts(cx: &mut TestAppContext) {
        let (_dir, window, workspace) = preview_docs(cx);
        step(cx, window, |window, cx| window.press("enter", cx));
        assert_eq!(bubble(cx, &workspace), None);
        assert_eq!(
            text(cx, &workspace),
            DOCUMENTED,
            "Enter keeps the change and inserts nothing"
        );
        assert!(cx.update(|cx| workspace.read(cx).editor.state().read(cx).is_editable()));

        step(cx, window, |window, cx| window.press("secondary-z", cx));
        assert_eq!(
            text(cx, &workspace),
            ORIGINAL,
            "one undo reverts the whole command"
        );
    }

    #[gpui_kit::test]
    fn typing_is_blocked_during_review(cx: &mut TestAppContext) {
        let (_dir, window, workspace) = preview_docs(cx);
        step(cx, window, |window, cx| {
            window.render_frame(cx);
            window.input("zzz", cx);
        });
        assert_eq!(text(cx, &workspace), DOCUMENTED);
        assert!(bubble(cx, &workspace).is_some_and(|b| b.is_preview()));
    }

    #[gpui_kit::test]
    fn tab_accepts(cx: &mut TestAppContext) {
        let (_dir, window, workspace) = preview_docs(cx);
        step(cx, window, |window, cx| window.press("tab", cx));
        assert_eq!(bubble(cx, &workspace), None);
        assert_eq!(
            text(cx, &workspace),
            DOCUMENTED,
            "Tab keeps the change and inserts nothing"
        );
    }

    #[gpui_kit::test]
    fn escape_rejects(cx: &mut TestAppContext) {
        let (_dir, window, workspace) = preview_docs(cx);
        step(cx, window, |window, cx| window.press("escape", cx));
        assert_eq!(bubble(cx, &workspace), None);
        assert_eq!(text(cx, &workspace), ORIGINAL);
        assert!(
            !cx.update(|cx| workspace.read(cx).dirty),
            "back to the saved text"
        );

        // The editor is usable again.
        step(cx, window, |window, cx| window.input("x", cx));
        assert!(text(cx, &workspace).contains('x'));
    }

    #[gpui_kit::test]
    fn undo_during_review_rejects(cx: &mut TestAppContext) {
        let (_dir, window, workspace) = preview_docs(cx);
        step(cx, window, |window, cx| window.press("secondary-z", cx));
        assert_eq!(bubble(cx, &workspace), None);
        assert_eq!(text(cx, &workspace), ORIGINAL);
    }

    #[gpui_kit::test]
    fn reject_keeps_earlier_edits(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("lib.rs");
        std::fs::write(&path, ORIGINAL).unwrap();
        let (window, workspace) = open(cx, &path);
        use_provider(
            cx,
            &workspace,
            Ok(r#"{"replace": "fn b() {}", "message": "Renamed."}"#),
        );
        step(cx, window, |window, cx| {
            window.render_frame(cx);
            window.press("secondary-down", cx);
            window.input("// kept\n", cx);
        });
        run_preset(cx, window, &workspace, 0..9, "simplify");
        assert_eq!(text(cx, &workspace), "fn b() {}\n// kept\n");
        step(cx, window, |window, cx| window.press("escape", cx));
        assert_eq!(
            text(cx, &workspace),
            "fn a() {}\n// kept\n",
            "only the command is undone"
        );
    }

    #[gpui_kit::test]
    fn unchanged_reply_only_shows_the_message(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("lib.rs");
        std::fs::write(&path, ORIGINAL).unwrap();
        let (window, workspace) = open(cx, &path);
        use_provider(
            cx,
            &workspace,
            Ok(r#"{"replace": "fn a() {}", "message": "Defines an empty function a."}"#),
        );
        run_preset(cx, window, &workspace, 0..9, "explain");
        assert_eq!(
            bubble(cx, &workspace),
            Some(Bubble::Message("Defines an empty function a.".into()))
        );
        assert_eq!(text(cx, &workspace), ORIGINAL);
        assert!(cx.update(|cx| workspace.read(cx).editor.state().read(cx).is_editable()));
    }

    #[gpui_kit::test]
    fn new_command_accepts_pending_change(cx: &mut TestAppContext) {
        let (_dir, window, workspace) = preview_docs(cx);
        step(cx, window, |window, cx| window.press("secondary-k", cx));
        let this_text = text(cx, &workspace);
        assert_eq!(this_text, DOCUMENTED);
        cx.update(|cx| {
            let this = workspace.read(cx);
            assert!(this.palette.is_some());
            assert!(this.editor.state().read(cx).is_editable());
        });
    }

    #[gpui_kit::test]
    fn waiting_tints_the_target_until_cancelled(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("lib.rs");
        std::fs::write(&path, ORIGINAL).unwrap();
        let (window, workspace) = open(cx, &path);
        use_provider(cx, &workspace, Ok(DOCS_REPLY));

        step(cx, window, |window, cx| {
            window.render_frame(cx);
            workspace.update(cx, |this, cx| {
                this.editor
                    .state()
                    .update(cx, |s, cx| s.set_selected_range(0..9, cx))
            });
            window.press("secondary-k", cx);
        });
        // Run without letting the request finish.
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.input("docs", cx);
            window.press("enter", cx);
        })
        .unwrap();
        cx.update(|cx| {
            let this = workspace.read(cx);
            assert!(this.run.as_ref().unwrap().bubble.is_running());
            assert_eq!(this.editor.highlighted_ranges(cx), vec![0..9]);
        });
        cx.update_window(window, |_, window, cx| window.press("escape", cx))
            .unwrap();
        cx.run_until_parked();
        cx.update(|cx| {
            let this = workspace.read(cx);
            assert!(this.run.is_none());
            assert!(this.editor.highlighted_ranges(cx).is_empty());
            assert_eq!(
                this.editor.text(cx),
                ORIGINAL,
                "a cancelled request changes nothing"
            );
        });
    }

    #[gpui_kit::test]
    fn provider_error_shows_and_fades(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("lib.rs");
        std::fs::write(&path, "fn a() {}\n").unwrap();
        let (window, workspace) = open(cx, &path);
        use_provider(cx, &workspace, Err("401 Unauthorized: invalid api key"));

        run_preset(cx, window, &workspace, 0..9, "docs");
        assert_eq!(
            bubble(cx, &workspace),
            Some(Bubble::Error("401 Unauthorized: invalid api key".into()))
        );
        cx.executor()
            .advance_clock(std::time::Duration::from_secs(5));
        cx.run_until_parked();
        assert_eq!(bubble(cx, &workspace), None);
    }

    #[gpui_kit::test]
    fn malformed_reply_is_an_error(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("lib.rs");
        std::fs::write(&path, "fn a() {}\n").unwrap();
        let (window, workspace) = open(cx, &path);
        use_provider(cx, &workspace, Ok("Sure, here is the code: fn a() {}"));

        run_preset(cx, window, &workspace, 0..9, "docs");
        assert!(matches!(bubble(cx, &workspace), Some(Bubble::Error(_))));
        assert_eq!(
            cx.update(|cx| workspace.read(cx).editor.text(cx)),
            "fn a() {}\n"
        );
    }

    #[gpui_kit::test]
    fn reply_for_a_changed_buffer_is_discarded(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("lib.rs");
        std::fs::write(&path, "fn a() {}\n").unwrap();
        let (window, workspace) = open(cx, &path);
        use_provider(cx, &workspace, Ok(r#"{"replace": "x", "message": "m"}"#));

        step(cx, window, |window, cx| {
            window.render_frame(cx);
            workspace.update(cx, |this, cx| {
                this.editor
                    .state()
                    .update(cx, |s, cx| s.set_selected_range(0..9, cx))
            });
            window.press("secondary-k", cx);
        });
        // Start the command; its request is queued but has not run yet.
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.input("docs", cx);
            window.press("enter", cx);
        })
        .unwrap();
        // Edit while the request is in flight, then let it finish.
        let ws = workspace.clone();
        step(cx, window, move |window, cx| {
            assert!(ws.read(cx).run.as_ref().unwrap().bubble.is_running());
            ws.update(cx, |this, cx| {
                this.editor.apply_edit(10..10, "// typed\n", window, cx);
            });
        });
        match bubble(cx, &workspace) {
            Some(Bubble::Error(message)) => assert!(message.contains("changed"), "{message}"),
            other => panic!("expected an error, got {other:?}"),
        }
    }

    #[gpui_kit::test]
    fn missing_api_key_is_reported(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("lib.rs");
        std::fs::write(&path, "fn a() {}\n").unwrap();
        let (window, workspace) = open(cx, &path);
        cx.update(|cx| {
            workspace.update(cx, |this, _| {
                this.provider = Err("OPENCODE_API_KEY is not set.".into())
            })
        });

        run_preset(cx, window, &workspace, 0..9, "docs");
        assert_eq!(
            bubble(cx, &workspace),
            Some(Bubble::Error("OPENCODE_API_KEY is not set.".into()))
        );
    }

    #[gpui_kit::test]
    fn escape_closes_palette(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("lib.rs");
        std::fs::write(&path, "fn a() {}\n").unwrap();
        let (window, workspace) = open(cx, &path);

        step(cx, window, |window, cx| {
            window.render_frame(cx);
            window.press("secondary-k", cx);
        });
        step(cx, window, |window, cx| {
            window.render_frame(cx);
            window.press("escape", cx);
        });
        step(cx, window, |window, cx| {
            let this = workspace.read(cx);
            assert!(this.palette.is_none());
            assert!(this.run.is_none());
            assert!(this.editor.state().focus_handle(cx).is_focused(window));
        });
    }

    /// A workspace whose commands file lives in a temp dir.
    fn open_with_commands(
        cx: &mut TestAppContext,
    ) -> (
        tempfile::TempDir,
        AnyWindowHandle,
        Entity<Workspace>,
        std::path::PathBuf,
    ) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("lib.rs");
        std::fs::write(&path, ORIGINAL).unwrap();
        let (window, workspace) = open(cx, &path);
        let commands = dir.path().join("config").join("commands.toml");
        let commands_for_ws = commands.clone();
        cx.update(|cx| workspace.update(cx, |this, _| this.commands_path = Some(commands_for_ws)));
        (dir, window, workspace, commands)
    }

    fn preset_names(cx: &mut TestAppContext, workspace: &Entity<Workspace>) -> Vec<String> {
        cx.update(|cx| {
            workspace
                .read(cx)
                .presets
                .iter()
                .map(|p| p.name.clone())
                .collect()
        })
    }

    #[gpui_kit::test]
    fn add_command_from_the_shortcut(cx: &mut TestAppContext) {
        let (_dir, window, workspace, commands) = open_with_commands(cx);
        step(cx, window, |window, cx| {
            window.render_frame(cx);
            window.press("secondary-shift-k", cx);
        });
        assert!(cx.update(|cx| workspace.read(cx).new_command.is_some()));
        step(cx, window, |window, cx| {
            window.render_frame(cx);
            window.input("Create controller", cx);
            window.press("tab", cx);
            window.input("Insert a REST controller at the cursor.", cx);
            window.press("secondary-2", cx);
            window.press("secondary-e", cx);
            window.press("secondary-enter", cx);
        });

        let source = std::fs::read_to_string(&commands).unwrap();
        let saved = jig_commands::presets::parse(&source).unwrap();
        assert_eq!(saved.len(), 1);
        assert_eq!(saved[0].name, "Create controller");
        assert_eq!(saved[0].prompt, "Insert a REST controller at the cursor.");
        assert_eq!(saved[0].scope, jig_commands::Scope::Cursor);
        assert!(saved[0].explore, "Cmd+E turned exploring on");
        assert!(
            preset_names(cx, &workspace).contains(&"Create controller".to_string()),
            "available in ⌘K at once"
        );
        step(cx, window, |window, cx| {
            let this = workspace.read(cx);
            assert!(this.new_command.is_none());
            assert!(this.editor.state().focus_handle(cx).is_focused(window));
            assert!(
                matches!(this.run.as_ref().map(|r| &r.bubble), Some(Bubble::Message(m)) if m.contains("Create controller"))
            );
            assert_eq!(
                this.editor.text(cx),
                ORIGINAL,
                "the file being edited is untouched"
            );
        });
    }

    #[gpui_kit::test]
    fn save_typed_instruction_as_command(cx: &mut TestAppContext) {
        let (_dir, window, workspace, commands) = open_with_commands(cx);
        step(cx, window, |window, cx| {
            window.render_frame(cx);
            window.press("secondary-k", cx);
        });
        step(cx, window, |window, cx| {
            window.render_frame(cx);
            window.input("wrap this in a tokio task", cx);
            window.press("secondary-enter", cx);
        });
        cx.update(|cx| {
            let this = workspace.read(cx);
            assert!(this.palette.is_none());
            assert!(
                this.new_command.is_some(),
                "the form replaces the command input"
            );
        });
        step(cx, window, |window, cx| {
            window.render_frame(cx);
            window.input("Spawn task", cx);
            window.press("secondary-enter", cx);
        });
        let saved =
            jig_commands::presets::parse(&std::fs::read_to_string(&commands).unwrap()).unwrap();
        assert_eq!(
            saved[0].prompt, "wrap this in a tokio task",
            "the typed text becomes the prompt"
        );
        assert_eq!(saved[0].scope, jig_commands::Scope::Selection);
    }

    #[gpui_kit::test]
    fn duplicate_name_keeps_the_form_open(cx: &mut TestAppContext) {
        let (_dir, window, workspace, commands) = open_with_commands(cx);
        std::fs::create_dir_all(commands.parent().unwrap()).unwrap();
        std::fs::write(&commands, "[[command]]\nname = \"Mine\"\nprompt = \"p\"\n").unwrap();
        step(cx, window, |window, cx| {
            window.render_frame(cx);
            window.press("secondary-shift-k", cx);
        });
        step(cx, window, |window, cx| {
            window.render_frame(cx);
            window.input("mine", cx);
            window.press("tab", cx);
            window.input("again", cx);
            window.press("secondary-enter", cx);
        });
        assert!(cx.update(|cx| workspace.read(cx).new_command.is_some()));
        assert_eq!(
            jig_commands::presets::parse(&std::fs::read_to_string(&commands).unwrap())
                .unwrap()
                .len(),
            1
        );
    }

    #[gpui_kit::test]
    fn escape_cancels_the_form(cx: &mut TestAppContext) {
        let (_dir, window, workspace, commands) = open_with_commands(cx);
        step(cx, window, |window, cx| {
            window.render_frame(cx);
            window.press("secondary-shift-k", cx);
        });
        step(cx, window, |window, cx| {
            window.render_frame(cx);
            window.input("Half typed", cx);
            window.press("escape", cx);
        });
        assert!(cx.update(|cx| workspace.read(cx).new_command.is_none()));
        assert!(!commands.exists());
    }

    #[gpui_kit::test]
    fn saving_the_commands_file_reloads_presets(cx: &mut TestAppContext) {
        let (_dir, window, workspace, commands) = open_with_commands(cx);
        cx.update(|cx| {
            cx.update_window(window, |_, window, cx| {
                workspace.update(cx, |this, cx| {
                    this.edit_commands(&super::EditCommands, window, cx)
                })
            })
            .unwrap()
        });
        cx.run_until_parked();
        assert!(commands.exists(), "created with a header");
        let ws = workspace.clone();
        step(cx, window, move |window, cx| {
            ws.update(cx, |this, cx| {
                let end = this.editor.text(cx).len();
                this.editor.apply_edit(
                    end..end,
                    "\n[[command]]\nname = \"From file\"\nprompt = \"p\"\n",
                    window,
                    cx,
                );
            });
            window.press("secondary-s", cx);
        });
        assert!(preset_names(cx, &workspace).contains(&"From file".to_string()));
    }

    /// Records the user message of each request.
    struct RecordingProvider(std::sync::Mutex<Vec<String>>);

    impl Provider for RecordingProvider {
        fn complete(&self, _: &str, user: &str) -> anyhow::Result<String> {
            self.0.lock().unwrap().push(user.to_string());
            Ok(r#"{"replace": "struct User;", "message": "Added."}"#.into())
        }
    }

    #[gpui_kit::test]
    fn note_reaches_the_model(cx: &mut TestAppContext) {
        let (_dir, window, workspace, commands) = open_with_commands(cx);
        std::fs::create_dir_all(commands.parent().unwrap()).unwrap();
        std::fs::write(
            &commands,
            "[[command]]\nname = \"Create model\"\nscope = \"cursor\"\nprompt = \"Insert a model.\"\ncomment = \"required\"\n",
        )
        .unwrap();
        let provider = Arc::new(RecordingProvider(Default::default()));
        let provider_for_ws: Arc<dyn Provider> = provider.clone();
        cx.update_window(window, |_, window, cx| {
            workspace.update(cx, |this, cx| {
                this.provider = Ok(provider_for_ws);
                this.reload_presets(window, cx);
            })
        })
        .unwrap();

        step(cx, window, |window, cx| {
            window.render_frame(cx);
            window.press("secondary-k", cx);
        });
        // Typed and confirmed in one frame, like a fast paste.
        step(cx, window, |window, cx| {
            window.render_frame(cx);
            window.input("model", cx);
            window.press("enter", cx);
        });
        step(cx, window, |window, cx| {
            window.input("User with an email", cx);
            window.press("enter", cx);
        });

        let sent = provider.0.lock().unwrap().clone();
        assert_eq!(sent.len(), 1);
        assert!(
            sent[0].contains("Instruction: Insert a model.\nNote: User with an email\n"),
            "{}",
            sent[0]
        );
    }

    #[gpui_kit::test]
    fn project_rules_go_with_every_command(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".git")).unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("JIG.md"), "Use thiserror for errors.\n").unwrap();
        let path = dir.path().join("src/lib.rs");
        std::fs::write(&path, ORIGINAL).unwrap();
        let (window, workspace) = open(cx, &path);
        let provider = Arc::new(RecordingProvider(Default::default()));
        let provider_for_ws: Arc<dyn Provider> = provider.clone();
        cx.update(|cx| workspace.update(cx, |this, _| this.provider = Ok(provider_for_ws)));

        run_preset(cx, window, &workspace, 0..9, "docs");
        let first = provider.0.lock().unwrap()[0].clone();
        assert!(
            first.starts_with("<project_rules>\nUse thiserror for errors.\n</project_rules>"),
            "{first}"
        );

        // Edits to JIG.md apply to the next command without a restart.
        step(cx, window, |window, cx| window.press("escape", cx));
        std::fs::write(dir.path().join("JIG.md"), "Prefer anyhow.\n").unwrap();
        run_preset(cx, window, &workspace, 0..9, "docs");
        let second = provider.0.lock().unwrap()[1].clone();
        assert!(second.contains("Prefer anyhow."), "{second}");
    }

    /// Reads `src/models.rs` through the tools and writes code that uses it.
    struct ExploringProvider(std::sync::Mutex<Vec<String>>);

    impl Provider for ExploringProvider {
        fn complete(&self, _: &str, _: &str) -> anyhow::Result<String> {
            anyhow::bail!("expected the tool path")
        }

        fn complete_with_tools(
            &self,
            system: &str,
            user: &str,
            tools: &dyn jig_ai::ToolHost,
            _: usize,
            on_step: &dyn Fn(String),
        ) -> anyhow::Result<String> {
            assert!(system.contains("read-only tools"));
            self.0.lock().unwrap().push(user.to_string());
            let input = serde_json::json!({ "path": "src/models.rs" });
            on_step(tools.describe("read_file", &input));
            let model = tools.call("read_file", &input);
            let secret = tools.call("read_file", &serde_json::json!({ "path": ".env" }));
            assert!(
                secret.starts_with("Error:"),
                "secrets stay hidden: {secret}"
            );
            let replace = format!("// uses: {}", model.trim());
            Ok(serde_json::json!({ "replace": replace, "message": "Used the model." }).to_string())
        }
    }

    #[gpui_kit::test]
    fn exploring_command_reads_the_project(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".git")).unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src/models.rs"), "pub struct User;\n").unwrap();
        std::fs::write(dir.path().join(".env"), "KEY=secret").unwrap();
        let path = dir.path().join("src/lib.rs");
        std::fs::write(&path, ORIGINAL).unwrap();
        let (window, workspace) = open(cx, &path);
        let provider = Arc::new(ExploringProvider(Default::default()));
        let provider_for_ws: Arc<dyn Provider> = provider.clone();
        cx.update(|cx| workspace.update(cx, |this, _| this.provider = Ok(provider_for_ws)));

        step(cx, window, |window, cx| {
            window.render_frame(cx);
            workspace.update(cx, |this, cx| {
                this.editor
                    .state()
                    .update(cx, |s, cx| s.set_selected_range(0..9, cx))
            });
            window.press("secondary-k", cx);
        });
        step(cx, window, |window, cx| {
            window.render_frame(cx);
            window.press("secondary-e", cx);
            window.input("simplify", cx);
            window.press("enter", cx);
        });

        assert_eq!(text(cx, &workspace), "// uses: pub struct User;\n");
        let user = provider.0.lock().unwrap()[0].clone();
        assert!(
            user.contains("File: src/lib.rs"),
            "the path is project-relative: {user}"
        );
    }
}
