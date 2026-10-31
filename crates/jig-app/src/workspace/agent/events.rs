//! What the agent says as a turn goes on, and the requests it makes, shown one at a time.

use super::*;

impl Workspace {
    pub(in crate::workspace) fn agent_run(&mut self, id: u64) -> Option<&mut AgentRun> {
        self.chat
            .as_mut()
            .filter(|run| run.id == id)
            .and_then(|run| run.agent.as_mut())
    }

    /// Handle one message from the agent's thread. Returns false once the
    /// run is over or gone.
    pub(super) fn on_agent_message(
        &mut self,
        id: u64,
        message: Message,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(agent) = self.agent_run(id) else {
            return false;
        };
        match message {
            Message::Started(session) => agent.session = Some(session),
            Message::Models(key, models) => jig_agent::agents::remember_models(&key, models),
            Message::Resumed(session, turns) => {
                agent.session = Some(session);
                agent.loading = false;
                (agent.entries, agent.drawings) = super::super::map_agent::transcript(turns);
                agent.scroll.scroll_to_bottom();
                let input = agent.input.clone();
                input.update(cx, |input, cx| input.focus(window, cx));
                self.sync_map_drawings(cx);
                cx.notify();
                return false;
            }
            Message::Event(AgentEvent::Step(step)) => agent.step.set(step),
            Message::Event(AgentEvent::Edit(edit)) => agent.queue.push_back(Request::Edit(edit)),
            Message::Event(AgentEvent::Permission(request)) => {
                agent.queue.push_back(Request::Permission(request))
            }
            Message::Event(AgentEvent::Question(request)) => {
                agent.queue.push_back(Request::Question(request))
            }
            Message::Event(AgentEvent::Drew { tool, .. }) if tool == jig_ai::map::PROJECT => {
                agent
                    .entries
                    .push(ChatEntry::Did("Read Jig's outline of the project".into()));
                cx.notify();
            }
            Message::Event(AgentEvent::Drew { tool, input })
                if tool == jig_ai::map::PROPOSE_DOMAINS =>
            {
                agent
                    .entries
                    .push(ChatEntry::Did("Proposed domains".into()));
                if agent.map_action.is_some() {
                    agent.proposed = Some(input);
                }
                cx.notify();
            }
            Message::Event(AgentEvent::Drew { tool, input }) => {
                self.agent_drew(id, &tool, &input, cx);
            }
            Message::Event(AgentEvent::Commands(commands)) => {
                let directory = agent.directory.clone();
                self.learn_commands(directory, commands, cx);
            }
            Message::Event(AgentEvent::Did(text)) => {
                // An edit that skipped review stays in sight, not folded
                // away with the tool calls.
                agent.entries.push(if text.ends_with("without asking") {
                    ChatEntry::Note(text)
                } else {
                    ChatEntry::Did(text)
                });
                agent.scroll.scroll_to_bottom();
                cx.notify();
            }
            Message::Event(AgentEvent::Subagent { id, description }) => {
                let (named, _, _) = agent.subagent(&id);
                *named = description;
                agent.step.set("Working with a subagent".into());
                cx.notify();
            }
            Message::Event(AgentEvent::SubagentDid { id, text }) => {
                agent.subagent(&id).1.push(text);
                cx.notify();
            }
            Message::Event(AgentEvent::SubagentDone { id, report }) => {
                *agent.subagent(&id).2 = Some(report);
                cx.notify();
            }
            Message::Event(AgentEvent::Done(text)) => {
                agent.finished = true;
                let action = agent.map_action.clone();
                let proposed = agent.proposed.take();
                let follow_up = agent.follow_up.take();
                self.finish_agent(text.clone(), window, cx);
                if let Some(action) = action {
                    self.answer_map_action(action, &text, proposed.as_ref(), window, cx);
                }
                // Already in the transcript, from when the user said it.
                if let Some(said) = follow_up {
                    self.start_turn(id, TurnInput::Prompt(said), window, cx);
                }
                return false;
            }
            Message::SignIn(needed) => {
                agent.finished = true;
                agent.follow_up = None;
                agent.loading = false;
                let said = needed.to_string();
                agent.sign_in = Some(PendingSignIn {
                    needed,
                    opened: false,
                });
                let mapped = agent.map_action.is_some();
                if mapped {
                    self.end_map_action(cx);
                }
                self.end_turn(ChatEntry::Note(said), window, cx);
                return false;
            }
            Message::Failed(error) => {
                agent.finished = true;
                agent.follow_up = None;
                agent.loading = false;
                if agent.map_action.is_some() {
                    self.end_map_action(cx);
                }
                let error = super::super::commands::shorten(&error);
                self.end_turn(ChatEntry::Error(error), window, cx);
                return false;
            }
        }
        // A request may have queued up behind one the user just settled.
        self.show_next_request(id, window, cx);
        true
    }

    /// What the agent just said it offers, for the palette's "/" and for
    /// replies.
    fn learn_commands(
        &mut self,
        directory: PathBuf,
        commands: Vec<jig_ai::agent::AgentCommand>,
        cx: &mut Context<Self>,
    ) {
        let agent = self.picked_agent(cx).key.clone();
        let commands = commands
            .into_iter()
            .map(|command| AgentCommandInfo {
                name: command.name,
                description: command.description,
                skill: command.skill,
            })
            .collect();
        self.catalog.update(cx, |catalog, cx| {
            catalog.learn_commands(directory, agent, commands, cx)
        });
    }

    /// Put the next queued request on screen, unless one is already there.
    pub(super) fn show_next_request(
        &mut self,
        id: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(agent) = self.agent_run(id) else {
            return;
        };
        if agent.pending.is_some() {
            return;
        }
        match agent.queue.pop_front() {
            None => {}
            Some(Request::Edit(edit)) if agent.accept_edits => {
                match self.write_agent_edit(&edit, window, cx) {
                    Ok(true) => self.show_next_request(id, window, cx),
                    // Unsaved changes in its file: left to `show_edit`, which
                    // tells the agent to leave it alone.
                    Ok(false) => {
                        if let Err(error) = self.show_edit(&edit, window, cx) {
                            self.answer_edit(&edit.id, false, Some(&error), cx);
                            self.show_next_request(id, window, cx);
                        }
                    }
                    Err(error) => {
                        self.answer_edit(&edit.id, false, Some(&error), cx);
                        self.show_next_request(id, window, cx);
                    }
                }
            }
            Some(Request::Edit(edit)) => {
                if let Err(error) = self.show_edit(&edit, window, cx) {
                    // Tell the agent, so it can try again.
                    self.answer_edit(&edit.id, false, Some(&error), cx);
                    self.show_next_request(id, window, cx);
                }
            }
            // Asked in the conversation, answered with the review keys in
            // the code.
            // Allowed always earlier in this conversation, a shell command
            // that only reads the project, or, for a Map action, anything
            // that only reads.
            Some(Request::Permission(request))
                if agent.always_allowed.contains(&request.title)
                    || (request.kind == Asking::Command
                        && command_only_reads(&request.detail, &agent.directory))
                    || (agent.map_action.is_some() && only_reads(&request, &agent.directory)) =>
            {
                if let Some(session) = agent.session.clone() {
                    cx.background_executor()
                        .spawn(async move {
                            let _ = session.reply(&request.id, true, None);
                        })
                        .detach();
                }
                self.show_next_request(id, window, cx);
            }
            // Saved with Enter, or answered in the reply box.
            Some(Request::Permission(request)) if request.task.is_some() => {
                agent.pending = Some(Pending::Permission(request));
                agent.scroll.scroll_to_bottom();
                let input = agent.input.clone();
                input.update(cx, |input, cx| input.focus(window, cx));
                cx.notify();
            }
            Some(Request::Permission(request)) => {
                agent.pending = Some(Pending::Permission(request));
                agent.scroll.scroll_to_bottom();
                self.focus_agent_or_editor(window, cx);
                cx.notify();
            }
            // Answered in the reply box.
            Some(Request::Question(request)) if !request.questions.is_empty() => {
                agent.pending = Some(Pending::Question {
                    request,
                    answers: Vec::new(),
                });
                agent.scroll.scroll_to_bottom();
                let input = agent.input.clone();
                input.update(cx, |input, cx| input.focus(window, cx));
                cx.notify();
            }
            Some(Request::Question(_)) => self.show_next_request(id, window, cx),
        }
    }

    fn finish_agent(&mut self, text: String, window: &mut Window, cx: &mut Context<Self>) {
        let text = if text.is_empty() {
            "Done.".to_string()
        } else {
            text
        };
        if let Some(run) = self.chat.as_mut() {
            run.bubble = Bubble::AgentDone(text.clone());
        }
        self.end_turn(ChatEntry::Agent(text), window, cx);
    }

    /// Close the agent's turn with `entry` and open the reply box.
    pub(super) fn end_turn(
        &mut self,
        entry: ChatEntry,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(agent) = self.chat.as_mut().and_then(|run| run.agent.as_mut()) else {
            return;
        };
        let touched = std::mem::take(&mut agent.touched);
        agent.entries.push(entry);
        agent.scroll.scroll_to_bottom();
        let input = agent.input.clone();
        if let Some(run) = self.chat.as_mut()
            && run.bubble.is_running()
        {
            run.bubble = Bubble::Message(String::new());
        }
        // Give OpenCode a moment to finish writing, then sync open tabs.
        cx.spawn_in(window, async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(300))
                .await;
            this.update_in(cx, |this, window, cx| {
                for path in &touched {
                    if let Some(ix) = this.tab_for(path) {
                        this.catch_up_with_disk(ix, window, cx);
                    }
                }
                this.refresh_tree(cx);
            })
            .ok();
        })
        .detach();

        self.editor().clear_highlights(cx);
        input.update(cx, |input, cx| input.focus(window, cx));
        cx.notify();
    }
}
