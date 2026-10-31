//! The conversation at the code: drawing it, moving and resizing it, and docking it to the side.

use super::*;

impl Workspace {
    /// The pin button: dock the conversation to the window's right edge, or
    /// float it at the code again.
    fn toggle_agent_chat_pin(&mut self, cx: &mut Context<Self>) {
        if let Some(agent) = self.chat.as_mut().and_then(|run| run.agent.as_mut()) {
            agent.frame.pinned = !agent.frame.pinned;
            self.agent.dock = agent.frame.pinned.then_some(agent.frame.width);
            cx.notify();
        }
    }

    /// Mouse handlers for the whole window, active while the docked
    /// conversation's edge is dragged: it keeps the right edge and takes
    /// what's left of the mouse.
    pub(in crate::workspace) fn agent_dock_drag_handlers(
        &self,
        root: Div,
        cx: &Context<Self>,
    ) -> Div {
        const MIN: f32 = 320.;
        root.when(self.agent.resizing_dock, |root| {
            root.cursor_col_resize()
                .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, window, cx| {
                    if !event.dragging() {
                        this.agent.resizing_dock = false;
                    } else {
                        let viewport = window.viewport_size().width;
                        // The column's padding on both sides.
                        let width = viewport - event.position.x - px(16.);
                        let width = width.clamp(px(MIN), (viewport * 0.7).max(px(MIN)));
                        if let Some(agent) = this.chat.as_mut().and_then(|run| run.agent.as_mut()) {
                            agent.frame.width = width;
                            this.agent.dock = Some(width);
                        }
                    }
                    cx.notify();
                }))
                .on_mouse_up(
                    MouseButton::Left,
                    cx.listener(|this, _, _, cx| {
                        this.agent.resizing_dock = false;
                        cx.notify();
                    }),
                )
        })
    }

    /// The conversation docked to the window's right edge, while pinned.
    pub(in crate::workspace) fn render_pinned_agent_chat(
        &self,
        cx: &Context<Self>,
    ) -> Option<AnyElement> {
        let agent = self.chat.as_ref()?.agent.as_ref()?;
        if self.mode == super::super::mode::Mode::Agent || !agent.frame.pinned {
            return None;
        }
        // Opaque like the code beside it: the window behind is translucent.
        // Its left edge resizes it, like the sidebar's right edge.
        let edge = div()
            .id("agent-dock-resize")
            .debug_selector(|| "agent-dock-resize".into())
            .absolute()
            .top_0()
            .bottom_0()
            .left(px(-3.))
            .w(px(6.))
            .cursor_col_resize()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    this.agent.resizing_dock = true;
                    cx.stop_propagation();
                }),
            );
        Some(
            div()
                .relative()
                .flex_none()
                .h_full()
                .p_2()
                .bg(jig_kit::theme::editor_surface(cx))
                .border_l_1()
                .border_color(cx.theme().title_bar_border)
                .children(self.render_agent_chat(cx))
                .child(edge)
                .into_any_element(),
        )
    }

    /// How much of the window's right a docked conversation takes.
    pub(in crate::workspace) fn docked_chat_width(&self) -> Pixels {
        if self.mode == super::super::mode::Mode::Agent {
            return px(0.);
        }
        self.chat
            .as_ref()
            .and_then(|run| run.agent.as_ref())
            .filter(|agent| agent.frame.pinned)
            .map_or(px(0.), |agent| agent.frame.width)
    }

    /// The conversation floating at the code, unless it's pinned.
    pub(in crate::workspace) fn floating_agent_chat_pinned(&self) -> bool {
        if self.mode == super::super::mode::Mode::Agent {
            return true;
        }
        self.chat
            .as_ref()
            .and_then(|run| run.agent.as_ref())
            .is_some_and(|agent| agent.frame.pinned)
    }

    /// Where the conversation opens: centred in the window's right half,
    /// top to bottom at the height it grows to, whatever code it's about.
    pub(in crate::workspace) fn agent_anchor(&self, window: &Window) -> Point<Pixels> {
        let frame = self
            .chat
            .as_ref()
            .and_then(|run| run.agent.as_ref())
            .map_or_else(Default::default, |agent| agent.frame.clone());
        self.right_centre(frame.width, frame.full_height(), window)
    }

    /// Pressed on the conversation's header: a drag may follow.
    fn grab_agent_chat(&mut self, event: &MouseDownEvent, cx: &mut Context<Self>) {
        let Some(run) = self.chat.as_mut() else {
            return;
        };
        let Some(agent) = run.agent.as_mut() else {
            return;
        };
        // From where it's drawn, which may differ from the anchor when it
        // was pushed in to fit the window.
        let origin = agent.frame.bounds.get().origin;
        agent.grab = Some(event.position - origin);
        run.anchor = origin;
        cx.stop_propagation();
    }

    /// Pressed on the conversation's corner grip: a resize may follow.
    fn grab_agent_chat_corner(&mut self, event: &MouseDownEvent, cx: &mut Context<Self>) {
        let Some(run) = self.chat.as_mut() else {
            return;
        };
        let Some(agent) = run.agent.as_mut() else {
            return;
        };
        let height = agent.frame.transcript_bounds.get().size.height;
        agent.resize = Some((event.position, size(agent.frame.width, height)));
        // It grows from where it's drawn.
        run.anchor = agent.frame.bounds.get().origin;
        cx.stop_propagation();
    }

    /// The conversation's corner follows the mouse while it's resized.
    pub(in crate::workspace) fn resize_agent_chat(
        &mut self,
        event: &DragMoveEvent<ChatResize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        const MIN: Size<Pixels> = Size {
            width: px(320.),
            height: px(60.),
        };
        let viewport = window.viewport_size();
        let Some(run) = self.chat.as_mut() else {
            return;
        };
        let Some(agent) = run.agent.as_mut() else {
            return;
        };
        let Some((start, from)) = agent.resize else {
            return;
        };
        // The resize cursor stays on wherever the mouse goes.
        cx.set_active_drag_cursor_style(CursorStyle::ResizeUpLeftDownRight, window);
        let by = event.event.position - start;
        // Room for the header and reply box below the transcript.
        let max = size(viewport.width - px(16.), viewport.height - px(180.));
        agent.frame.width = (from.width + by.x).clamp(MIN.width, max.width.max(MIN.width));
        agent.frame.transcript_height =
            Some((from.height + by.y).clamp(MIN.height, max.height.max(MIN.height)));
        cx.notify();
    }

    /// The conversation follows the mouse while its header is dragged.
    pub(in crate::workspace) fn drag_agent_chat(
        &mut self,
        event: &DragMoveEvent<ChatDrag>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.set_active_drag_cursor_style(CursorStyle::ClosedHand, window);
        let Some(run) = self.chat.as_mut() else {
            return;
        };
        let Some(agent) = run.agent.as_mut() else {
            return;
        };
        let Some(grab) = agent.grab else {
            return;
        };
        run.anchor = event.event.position - grab;
        agent.moved = true;
        cx.notify();
    }

    /// The conversation, while the run is an agent's.
    pub(in crate::workspace) fn render_agent_chat(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let run = self.chat.as_ref()?;
        let agent = run.agent.as_ref()?;
        let status = match (&agent.pending, &run.bubble) {
            _ if agent.loading => ChatStatus::Loading,
            _ if let Some(sign_in) = agent.sign_in.as_ref() => ChatStatus::SignIn {
                options: sign_in
                    .needed
                    .methods
                    .iter()
                    .map(|method| {
                        if method.description.is_empty() {
                            method.name.clone()
                        } else {
                            format!("{}: {}", method.name, method.description)
                        }
                    })
                    .collect(),
                opened: sign_in.opened,
            },
            (
                Some(Pending::Permission(PermissionRequest {
                    task: Some(task), ..
                })),
                _,
            ) => ChatStatus::SaveTask {
                title: task.title.clone(),
                todos: task.todos.clone(),
            },
            (Some(Pending::Permission(request)), _) => ChatStatus::Asking {
                title: request.title.clone(),
                detail: request.detail.clone(),
                always: request
                    .always
                    .clone()
                    .unwrap_or_else(|| "Allow in this conversation".into()),
            },
            (Some(Pending::Question { request, answers }), _) => {
                let question = &request.questions[answers.len()];
                ChatStatus::Question {
                    question: question.question.clone(),
                    options: question.options.clone(),
                    position: (answers.len() + 1, request.questions.len()),
                }
            }
            (_, bubble) => match bubble {
                Bubble::Running { started, step, .. } => ChatStatus::Working {
                    started: *started,
                    step: step.clone(),
                },
                Bubble::Preview { message, removed } => ChatStatus::Reviewing {
                    path: message.clone(),
                    removed: removed.clone(),
                },
                _ => ChatStatus::Waiting,
            },
        };
        Some(
            Conversation::new(
                format!("{} · {}", agent.label, agent.project),
                linked_entries(&agent.entries, agent.directory()),
                status,
                agent.input.clone(),
                agent.scroll.clone(),
                agent.frame.clone(),
            )
            .full_page(self.mode == super::super::mode::Mode::Agent)
            .on_submit(cx.listener(|this, _, window, cx| {
                if !this.answer_agent_question(None, window, cx)
                    && !this.answer_task_question(window, cx)
                {
                    this.reply_to_agent(window, cx);
                }
            }))
            .on_grab(cx.listener(|this, event, _, cx| this.grab_agent_chat(event, cx)))
            .on_resize_grab(
                cx.listener(|this, event, _, cx| this.grab_agent_chat_corner(event, cx)),
            )
            .on_pin(cx.listener(|this, _, _, cx| this.toggle_agent_chat_pin(cx)))
            .models(
                self.conversation_models(cx).1,
                cx.listener(|this, _, _, cx| this.toggle_conversation_models(cx)),
                cx.listener(|this, index: &usize, _, cx| this.pick_conversation_model(*index, cx)),
            )
            .on_show_edit(cx.listener(|this, _, window, cx| {
                this.show_agent_edit(window, cx);
            }))
            .on_history(cx.listener(|this, _, window, cx| {
                this.open_agent_conversations(&OpenAgentConversations, window, cx)
            }))
            .on_decide(cx.listener(|this, button: &usize, window, cx| {
                this.decide_agent_request(*button, window, cx)
            }))
            .on_option(cx.listener(|this, index: &usize, window, cx| {
                if this.signing_in() {
                    this.sign_in_with(*index, window, cx);
                } else {
                    this.answer_agent_question(Some(*index), window, cx);
                }
            }))
            .on_drawing(
                cx.listener(|this, (at, action): &(usize, DrawingAction), window, cx| {
                    this.act_on_drawing(*at, *action, window, cx)
                }),
            )
            .on_link({
                let this = cx.weak_entity();
                std::sync::Arc::new(
                    move |link: &SharedString, window: &mut Window, cx: &mut App| {
                        let link = link.to_string();
                        this.update(cx, |this, cx| this.open_agent_link(&link, window, cx))
                            .ok();
                    },
                )
            })
            .into_any_element(),
        )
    }
}

/// The transcript with the files the agent names but didn't link made
/// links, so every file it mentions opens with a click.
fn linked_entries(entries: &[ChatEntry], project: &Path) -> Vec<ChatEntry> {
    let file = |written: &str| {
        let path = Path::new(written);
        let relative = path.strip_prefix(project).unwrap_or(path);
        let relative = relative.strip_prefix("./").unwrap_or(relative);
        (relative.is_relative() && project.join(relative).is_file())
            .then(|| relative.to_string_lossy().replace(' ', "%20"))
    };
    entries
        .iter()
        .map(|entry| match entry {
            ChatEntry::Agent(text) => ChatEntry::Agent(link_files(text, file)),
            ChatEntry::Subagent {
                id,
                description,
                calls,
                report,
            } => ChatEntry::Subagent {
                id: id.clone(),
                description: description.clone(),
                calls: calls.clone(),
                report: report.as_deref().map(|report| link_files(report, file)),
            },
            entry => entry.clone(),
        })
        .collect()
}
