//! The one window: a single document in a single editor.

use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

use gpui_kit::component::input::{Editor, EditorState, InputEvent};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use jig_commands::{CommandPalette, Invocation, PaletteEvent, Preset, presets};
use jig_editor::{EditorHandle, KitEditor};

use crate::document::Document;

actions!(jig, [Quit, Open, Save, SaveAs, CloseWindow, OpenCommand]);

pub struct Workspace {
    document: Document,
    editor: KitEditor,
    dirty: bool,
    presets: Rc<Vec<Preset>>,
    palette: Option<OpenPalette>,
    /// The last command run. Until the AI round trip lands (M3), running a
    /// command only flashes its target range.
    last_invocation: Option<Invocation>,
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
            last_invocation: None,
            _editor_events: subscription,
        };
        this.update_title(window);
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

    fn open_command(&mut self, _: &OpenCommand, window: &mut Window, cx: &mut Context<Self>) {
        if self.palette.is_some() {
            return;
        }
        let has_selection = !self.editor.selection(cx).is_empty();
        let anchor = self
            .editor
            .anchor_point(cx)
            .map(|point| point + gpui_kit::point(px(-8.), px(4.)))
            // The cursor is scrolled out of view: open near the top instead.
            .unwrap_or(gpui_kit::point(px(48.), px(48.)));
        let presets = self.presets.clone();
        let view = cx.new(|cx| CommandPalette::new(presets, has_selection, window, cx));
        let events = cx.subscribe_in(
            &view,
            window,
            |this, _, event: &PaletteEvent, window, cx| {
                this.close_palette(window, cx);
                if let PaletteEvent::Run(invocation) = event {
                    this.run_command(invocation.clone(), window, cx);
                }
            },
        );
        self.palette = Some(OpenPalette {
            view,
            anchor,
            _events: events,
        });
        cx.notify();
    }

    fn close_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.palette.take().is_some() {
            self.editor.focus(window, cx);
            cx.notify();
        }
    }

    fn run_command(&mut self, invocation: Invocation, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.editor.text(cx);
        let target =
            invocation
                .scope
                .target(&text, self.editor.selection(cx), self.editor.cursor(cx));
        self.last_invocation = Some(invocation);

        // Placeholder until M3: flash the range the command would replace.
        self.editor
            .highlight(vec![(target, hsla(0.6, 0.8, 0.55, 0.22))], cx);
        cx.spawn_in(window, async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(700))
                .await;
            this.update(cx, |this, cx| this.editor.clear_highlights(cx))
                .ok();
        })
        .detach();
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
    ]
}

impl Render for Workspace {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .on_action(cx.listener(Self::quit))
            .on_action(cx.listener(Self::open))
            .on_action(cx.listener(Self::save))
            .on_action(cx.listener(Self::save_as))
            .on_action(cx.listener(Self::close_window))
            .on_action(cx.listener(Self::open_command))
            .child(Editor::new(self.editor.state()).bordered(false).size_full())
            .when_some(self.palette.as_ref(), |this, palette| {
                this.child(deferred(
                    anchored()
                        .position(palette.anchor)
                        .snap_to_window_with_margin(px(8.))
                        .child(palette.view.clone()),
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
    use jig_editor::EditorHandle;

    use super::{Workspace, key_bindings};

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

    #[gpui_kit::test]
    fn cmd_k_runs_a_preset_and_returns_focus(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("lib.rs");
        std::fs::write(&path, "fn a() {}\n").unwrap();
        let (window, workspace) = open(cx, &path);

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
            assert!(workspace.read(cx).palette.is_some());
            window.render_frame(cx);
            window.input("docs", cx);
            window.press("enter", cx);
        });
        step(cx, window, |window, cx| {
            let this = workspace.read(cx);
            assert!(
                this.palette.is_none(),
                "running a command closes the palette"
            );
            let invocation = this.last_invocation.clone().expect("a command ran");
            assert_eq!(invocation.name.as_deref(), Some("Add docs"));
            assert!(
                this.editor.state().focus_handle(cx).is_focused(window),
                "focus returns to the editor"
            );
            assert_eq!(
                this.editor.text(cx),
                "fn a() {}\n",
                "M2 does not edit the buffer"
            );
        });
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
            assert!(this.last_invocation.is_none());
            assert!(this.editor.state().focus_handle(cx).is_focused(window));
        });
    }
}
