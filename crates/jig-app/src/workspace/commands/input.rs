//! The ⌘K input: its button, opening it, and starting a conversation from what's typed.

use super::*;

impl Workspace {
    /// A button in the title bar that opens the agent input, for those who
    /// reach for the mouse or don't know ⌘K yet. Only with a file open.
    pub(in crate::workspace) fn render_command_button(
        &self,
        cx: &Context<Self>,
    ) -> Option<AnyElement> {
        if self.home || self.mode == super::super::mode::Mode::Agent {
            return None;
        }
        let theme = cx.theme();
        let accent = jig_commands::surface::agent_accent(cx);
        // Pressed while the palette is open: it toggles. It glows as the
        // palette opens, so the two read as one.
        let opened = self.palette.as_ref().map(|palette| palette.serial);
        let shortcut = if cfg!(target_os = "macos") {
            "⌘K"
        } else {
            "Ctrl K"
        };
        Some(
            h_flex()
                .id("open-command")
                .debug_selector(|| "open-command".into())
                .relative()
                .flex_none()
                .ml_2()
                .h(px(26.))
                .px_2()
                .gap_1p5()
                .rounded(px(6.))
                .text_size(px(12.5))
                .text_color(theme.foreground.opacity(0.85))
                // Behind the label: bright as the palette opens, settling to
                // the pressed tint.
                .children(opened.map(|serial| {
                    div()
                        .absolute()
                        .top_0()
                        .left_0()
                        .size_full()
                        .rounded(px(6.))
                        .with_animation(
                            ("agent-button-glow", serial),
                            Animation::new(Duration::from_millis(420))
                                .with_easing(ease_out_quint()),
                            move |glow, delta| glow.bg(accent.opacity(0.38 - 0.22 * delta)),
                        )
                }))
                .when(opened.is_some(), |s| s.text_color(accent))
                .when(opened.is_none(), |s| {
                    s.hover(|s| s.bg(theme.foreground.opacity(0.08)))
                })
                .child(
                    jig_commands::surface::agent_icon()
                        .size(px(13.))
                        .flex_none()
                        .text_color(accent),
                )
                .child(div().flex_none().whitespace_nowrap().child("Agent"))
                .child(
                    div()
                        .flex_none()
                        .text_size(px(11.))
                        .text_color(theme.muted_foreground)
                        .child(shortcut),
                )
                // Otherwise the title bar takes the press as a window drag.
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(
                    cx.listener(|this, _, window, cx| this.open_command(&OpenCommand, window, cx)),
                )
                .into_any_element(),
        )
    }

    pub(in crate::workspace) fn open_command(
        &mut self,
        _: &OpenCommand,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let map = self.map_showing().cloned();
        if self.home && map.is_none() && self.mode != super::super::mode::Mode::Agent {
            return;
        }
        // ⌘K and the Agent button toggle it.
        if self.palette.is_some() {
            return self.close_palette(window, cx);
        }
        self.close_quick_open(cx);
        self.close_find_in_files(cx);
        // Its matches would muddle the change's highlights.
        self.close_find_panel(cx);
        self.note = None;
        let selection = self.editor().selection(cx);
        // On the Map, beside the boxes selected; in the code, beside the
        // selection, or the cursor's line.
        let (has_selection, anchor) = match &map {
            Some(view) => {
                let beside = view.read(cx).question().beside;
                (beside.is_some(), self.map_chat_anchor(beside, window))
            }
            None => (
                !selection.is_empty(),
                // Its top where the conversation's will be, so that opens
                // where it was asked.
                self.right_centre(
                    px(jig_commands::palette::WIDTH),
                    jig_commands::chat::ChatFrame::default().full_height(),
                    window,
                ),
            ),
        };
        let models = self.model_menu(cx);
        let view = cx.new(|cx| CommandPalette::new(has_selection, window, cx).with_models(models));
        let events = cx.subscribe_in(
            &view,
            window,
            |this, palette, event: &PaletteEvent, window, cx| {
                if let PaletteEvent::PickModel(choice) = event {
                    return this.pick_model(choice.clone(), palette.clone(), cx);
                }
                this.close_palette(window, cx);
                match event {
                    PaletteEvent::Run(text) if this.map_showing().is_some() => {
                        this.start_map_conversation(None, text.clone(), window, cx)
                    }
                    PaletteEvent::Run(text) if this.mode == super::super::mode::Mode::Agent => {
                        this.submit_agent_mode_prompt(text.clone(), window, cx);
                    }
                    PaletteEvent::Run(text) => {
                        this.start_conversation(Ask::new(text.clone()), window, cx)
                    }
                    PaletteEvent::Resume(id) => this.resume_agent(id.clone(), window, cx),
                    PaletteEvent::RunAgentCommand { name, arguments } => {
                        this.run_agent_command(name.clone(), arguments.clone(), window, cx)
                    }
                    PaletteEvent::PickModel(_) | PaletteEvent::Dismissed => {}
                }
            },
        );
        self.load_agent_catalog(Some(view.clone()), cx);
        self.palette_openings += 1;
        self.palette = Some(OpenPalette {
            view,
            anchor,
            serial: self.palette_openings,
            _events: events,
        });
        cx.notify();
    }

    pub(in crate::workspace) fn close_palette(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.palette.take().is_some() {
            self.focus_code(window, cx);
            cx.notify();
        }
    }

    /// Start a conversation about the selection, or the cursor's line, in
    /// place of the one on screen.
    pub(in crate::workspace) fn start_conversation(
        &mut self,
        ask: Ask,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Two changes under review in one buffer would tangle their undo.
        if self.agent_edit_here() {
            return self.show_note(
                "Accept or reject the agent's edit first.".into(),
                window,
                cx,
            );
        }
        self.switch_mode(super::super::mode::Mode::Code, cx);
        let text = self.editor().text(cx);
        let selection = self.editor().selection(cx);
        let target = if selection.is_empty() {
            line_at(&text, selection.start)
        } else {
            selection
        };
        let request = jig_ai::agent::Prompt {
            instruction: ask.instruction,
            language: self.editor().language(cx),
            file_name: self.document().path.as_deref().map(|path| {
                // Relative to the project, so the agent knows where the
                // file sits.
                let root = jig_kit::project::root_for(path);
                path.strip_prefix(&root)
                    .unwrap_or(path)
                    .to_string_lossy()
                    .into_owned()
            }),
            diagnostics: if ask.diagnostics {
                self.problems_on_lines(&text, target.clone(), cx)
            } else {
                Vec::new()
            },
            task: ask.task,
            text,
            target: target.clone(),
        };
        self.close_chat(window, cx);
        self.next_run_id += 1;
        let id = self.next_run_id;
        self.editor()
            .highlight(vec![(target.clone(), working_color(cx))], cx);
        self.chat = Some(CommandRun {
            id,
            bubble: Bubble::Message(String::new()),
            anchor: self.agent_anchor(window),
            preview: None,
            agent: None,
            _task: None,
        });
        self.run_agent(id, ask.name, request, window, cx);
    }

    /// The project files the agent reads besides the code, e.g.
    /// "AGENTS.md", by name.
    pub(in crate::workspace) fn context_names(&self) -> Vec<String> {
        self.document()
            .path
            .as_deref()
            .map(jig_kit::project::context_names)
            .unwrap_or_default()
    }
}
