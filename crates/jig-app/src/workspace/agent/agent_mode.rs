//! Agent mode: the conversation filling the window, or the recent ones and a composer.

use super::*;

impl Workspace {
    /// With no conversation selected, show the project's recent conversations.
    pub(in crate::workspace) fn show_agent_mode(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self
            .chat
            .as_ref()
            .and_then(|run| run.agent.as_ref())
            .is_none()
        {
            if let Some(directory) = self.agent_project(cx)
                && self
                    .agent
                    .home_draft
                    .as_ref()
                    .is_none_or(|(project, _)| *project != directory)
            {
                let input = cx.new(|cx| {
                    TextareaState::new(window, cx)
                        .placeholder("Ask the agent about your project…")
                        .auto_grow(1, 8)
                        .soft_wrap(true)
                });
                self.agent.home_draft = Some((directory, input));
            }
            self.load_agent_catalog(None, cx);
        }
        self.focus_agent_mode(window, cx);
        cx.notify();
    }

    fn ensure_agent_conversation(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self
            .chat
            .as_ref()
            .and_then(|run| run.agent.as_ref())
            .is_some()
        {
            return;
        }
        let Some(id) = self.start_agent_run(window, cx) else {
            return;
        };
        let directory = self.agent_project(cx).expect("start_agent_run checked");
        let mut agent = self.new_agent_run(None, String::new(), directory, window, cx);
        agent.entries.clear();
        agent.input.update(cx, |input, cx| {
            input.set_placeholder("Ask the agent about your project…", window, cx)
        });
        if let Some(run) = self.chat.as_mut().filter(|run| run.id == id) {
            run.agent = Some(agent);
        }
        self.focus_agent_mode(window, cx);
        cx.notify();
    }

    pub(in crate::workspace) fn focus_agent_mode(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(agent) = self.chat.as_ref().and_then(|run| run.agent.as_ref()) {
            if agent.loading
                || matches!(
                    &agent.pending,
                    Some(Pending::Permission(_)) | Some(Pending::Edit(_))
                )
            {
                agent.page_focus.focus(window, cx);
                return;
            }
            agent.input.update(cx, |input, cx| input.focus(window, cx));
        } else if let Some(input) = self.agent_home_input(cx) {
            input.update(cx, |input, cx| input.focus(window, cx));
        } else {
            self.home_focus.focus(window, cx);
        }
    }

    pub(super) fn focus_agent_or_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.mode == super::super::mode::Mode::Agent {
            self.focus_agent_mode(window, cx);
        } else {
            self.focus_editor(window, cx);
        }
    }

    pub(in crate::workspace) fn submit_agent_mode_prompt(
        &mut self,
        text: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if text.trim().is_empty() {
            return;
        }
        if self.agent_busy() {
            return self.show_note(
                "Wait for the agent, or stop its turn first.".into(),
                window,
                cx,
            );
        }
        self.ensure_agent_conversation(window, cx);
        if let Some(agent) = self.chat.as_ref().and_then(|run| run.agent.as_ref()) {
            agent
                .input
                .update(cx, |input, cx| input.set_value(text, window, cx));
        }
        self.reply_to_agent(window, cx);
    }

    fn agent_home_input(&self, cx: &App) -> Option<&Entity<TextareaState>> {
        let directory = self.agent_project(cx)?;
        self.agent
            .home_draft
            .as_ref()
            .filter(|(project, _)| *project == directory)
            .map(|(_, input)| input)
    }

    fn submit_agent_home_message(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(input) = self.agent_home_input(cx).cloned() else {
            return;
        };
        let text = input.read(cx).value().to_string();
        if text.trim().is_empty() {
            return;
        }
        self.submit_agent_mode_prompt(text, window, cx);
        if self
            .chat
            .as_ref()
            .and_then(|run| run.agent.as_ref())
            .is_some_and(|agent| agent.last_input.is_some())
        {
            input.update(cx, |input, cx| input.set_value("", window, cx));
        }
    }

    pub(super) fn agent_busy(&self) -> bool {
        self.chat
            .as_ref()
            .and_then(|run| run.agent.as_ref())
            .is_some_and(|agent| !agent.finished || agent.loading || agent.pending.is_some())
    }

    pub(in crate::workspace) fn new_agent_conversation(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.agent_busy() || self.chat.as_ref().is_some_and(|run| run.preview.is_some()) {
            return self.show_note(
                "Finish or stop the current conversation first.".into(),
                window,
                cx,
            );
        }
        self.close_chat(window, cx);
        self.ensure_agent_conversation(window, cx);
    }

    pub(in crate::workspace) fn render_agent_mode(&self, cx: &Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let busy = self.agent_busy();
        v_flex()
            .id("agent-mode")
            // Keep the shared Enter/Tab/Escape review keys active when a request
            // owns focus and the message editor is not on screen.
            .key_context("Input")
            .debug_selector(|| "agent-mode".into())
            .track_focus(
                self.chat
                    .as_ref()
                    .and_then(|run| run.agent.as_ref())
                    .map_or(&self.home_focus, |agent| &agent.page_focus),
            )
            .flex_1()
            .min_h_0()
            .w_full()
            .child(
                h_flex()
                    .flex_wrap()
                    .px_4()
                    .py_2()
                    .gap_2()
                    .border_b_1()
                    .border_color(theme.border)
                    .child(
                        div()
                            .text_sm()
                            .text_color(theme.muted_foreground)
                            .child("Agent"),
                    )
                    .child(div().flex_1())
                    .child(
                        Button::new("agent-mode-choose")
                            .disabled(
                                self.chat
                                    .as_ref()
                                    .and_then(|run| run.agent.as_ref())
                                    .is_some_and(|agent| {
                                        !agent.entries.is_empty() || agent.loading
                                    }),
                            )
                            .label("Choose agent")
                            .small()
                            .ghost()
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.open_command(&OpenCommand, window, cx);
                                if let Some(palette) = &this.palette {
                                    palette.view.update(cx, |palette, cx| {
                                        palette.start_picking(window, cx)
                                    });
                                }
                            })),
                    )
                    .child(
                        Button::new("agent-mode-history")
                            .label("Conversations")
                            .small()
                            .ghost()
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.open_agent_conversations(&OpenAgentConversations, window, cx)
                            })),
                    )
                    .when(
                        self.chat
                            .as_ref()
                            .and_then(|run| run.agent.as_ref())
                            .is_some(),
                        |this| {
                            this.child(
                                Button::new("agent-mode-new")
                                    .label("New conversation")
                                    .small()
                                    .ghost()
                                    .disabled(busy)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.new_agent_conversation(window, cx)
                                    })),
                            )
                        },
                    )
                    .when(busy, |this| {
                        this.child(
                            Button::new("agent-mode-stop")
                                .label("Stop")
                                .small()
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.stop_agent_turn(window, cx);
                                })),
                        )
                    }),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .flex()
                    .justify_center()
                    .child(
                        div().h_full().w_full().max_w(px(960.)).min_w_0().child(
                            self.render_agent_chat(cx)
                                .unwrap_or_else(|| self.render_recent_conversations(cx)),
                        ),
                    ),
            )
            .into_any_element()
    }

    fn render_recent_conversations(&self, cx: &Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let directory = self.agent_project(cx);
        let agent = self.picked_agent(cx);
        let catalog = self.catalog.read(cx);
        let mut recent: Vec<_> = directory
            .as_deref()
            .and_then(|directory| catalog.for_project(directory, &agent.key))
            .into_iter()
            .flat_map(|catalog| &catalog.conversations)
            .collect();
        recent.sort_by_key(|conversation| std::cmp::Reverse(conversation.updated));
        let empty = recent.is_empty();
        let now = SystemTime::now();
        v_flex()
            .id("agent-recents")
            .debug_selector(|| "agent-recents".into())
            .size_full()
            .p_4()
            .gap_3()
            .child(div().text_size(px(20.)).child("Recent conversations"))
            .child(
                div()
                    .text_sm()
                    .text_color(theme.muted_foreground)
                    .child(format!(
                        "{} · {}",
                        agent.name,
                        directory
                            .as_ref()
                            .and_then(|path| path.file_name())
                            .map(|name| name.to_string_lossy())
                            .unwrap_or_default()
                    )),
            )
            .when(catalog.loading(), |this| {
                this.child(
                    div()
                        .text_sm()
                        .text_color(theme.muted_foreground)
                        .child("Loading conversations…"),
                )
            })
            .when_some(catalog.error(), |this, error| {
                this.child(
                    v_flex()
                        .gap_2()
                        .child(
                            div()
                                .text_sm()
                                .text_color(theme.muted_foreground)
                                .child(error.clone()),
                        )
                        .child(
                            Button::new("agent-recents-retry")
                                .label("Try again")
                                .small()
                                .on_click(
                                    cx.listener(|this, _, _, cx| this.load_agent_catalog(None, cx)),
                                ),
                        ),
                )
            })
            .when(
                empty && !catalog.loading() && catalog.error().is_none(),
                |this| {
                    this.child(
                        div()
                            .text_sm()
                            .text_color(theme.muted_foreground)
                            .child("No conversations yet. Type a message below to start one."),
                    )
                },
            )
            .child(
                v_flex()
                    .id("agent-recents-list")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .track_scroll(&self.agent.recent_scroll)
                    .gap_1()
                    .children(recent.into_iter().take(30).enumerate().map(
                        |(index, conversation)| {
                            let id = conversation.id.clone();
                            let title = conversation_title(conversation);
                            div()
                                .debug_selector(move || format!("agent-recent-{index}"))
                                .child(
                                    Button::new(("agent-recent", index))
                                        .label(format!(
                                            "{title} · {}",
                                            ago(now, conversation.updated)
                                        ))
                                        .w_full()
                                        .ghost()
                                        .on_click(cx.listener(move |this, _, window, cx| {
                                            this.resume_agent(id.clone(), window, cx)
                                        })),
                                )
                        },
                    )),
            )
            .when_some(self.agent_home_input(cx), |this, input| {
                this.child(
                    v_flex()
                        .flex_none()
                        .gap_2()
                        .child(jig_commands::chat::message_input(
                            input,
                            cx.listener(|this, _, window, cx| {
                                this.submit_agent_home_message(window, cx)
                            }),
                            cx,
                        ))
                        .child(jig_commands::surface::hint(
                            "enter to send · shift-enter for a new line",
                            cx,
                        )),
                )
            })
            .into_any_element()
    }
}

pub(super) fn conversation_title(conversation: &jig_ai::agent::Conversation) -> String {
    conversation
        .title
        .clone()
        .unwrap_or_else(|| "Earlier conversation".into())
}

/// How long ago `then` was, roughly: "just now", "5 min ago", "yesterday".
pub(super) fn ago(now: SystemTime, then: SystemTime) -> String {
    let seconds = now.duration_since(then).unwrap_or_default().as_secs();
    let (minutes, hours, days) = (seconds / 60, seconds / 3600, seconds / 86_400);
    match () {
        _ if minutes < 1 => "just now".into(),
        _ if hours < 1 => format!("{minutes} min ago"),
        _ if days < 1 => format!("{hours} h ago"),
        _ if days < 2 => "yesterday".into(),
        _ if days < 30 => format!("{days} days ago"),
        _ => format!("{} months ago", days / 30),
    }
}
