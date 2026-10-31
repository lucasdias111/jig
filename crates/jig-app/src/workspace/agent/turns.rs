//! Sending the agent a turn: a message, the selection, a reply or a command, and signing in first when it asks.

use super::*;

impl Workspace {
    /// Send `input` to the agent, in the run's session once there is one,
    /// and follow what it does until it's done.
    pub(in crate::workspace) fn start_turn(
        &mut self,
        id: u64,
        input: TurnInput,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // The agent reads the disk, so it must see what's on screen.
        if self.tab().dirty
            && let Some(path) = self.document().path.clone()
        {
            self.save_to(&path, window, cx);
        }
        // Learn the commands, so a reply can run one.
        if self.catalog.read(cx).catalog().is_none() {
            self.load_agent_catalog(None, cx);
        }
        // Read on this thread: the keys and settings live in the app.
        let pick = self.agent_run(id).and_then(|agent| agent.pick.clone());
        let choice = jig_agent::agent::choice_for(pick.as_ref(), cx);
        let Some(agent) = self.agent_run(id) else {
            return;
        };
        let session = agent.session.clone();
        let directory = agent.directory.clone();
        let title = agent.title.clone();
        let map_action = agent.map_action.clone();
        agent.proposed = None;
        agent.last_input = Some(input.clone());
        let permissions = jig_settings::get(cx).agent.permissions;
        agent.step = LiveStep::default();
        agent.step.set(if session.is_some() {
            "Thinking".into()
        } else {
            "Starting the agent".into()
        });
        agent.started = std::time::Instant::now();
        agent.finished = false;
        let bubble = Bubble::Running {
            label: agent.label.clone(),
            started: agent.started,
            context: agent.context.clone(),
            step: agent.step.clone(),
        };

        let (sender, mut messages) = futures::channel::mpsc::unbounded();
        let turn = move || {
            let send = |message| {
                let _ = sender.unbounded_send(message);
            };
            let result = (|| {
                let choice = choice.map_err(anyhow::Error::msg)?;
                let session = match session {
                    Some(session) => session,
                    None => {
                        let backend = jig_agent::agent::connect(&choice)?;
                        let session =
                            backend.create(&directory, &title, &permissions, choice.acp_model())?;
                        if let Some(models) = backend.models() {
                            send(Message::Models(choice.key().to_string(), models));
                        }
                        send(Message::Started(session.clone()));
                        session
                    }
                };
                let (prompt, command) = match input {
                    TurnInput::Prompt(prompt) => (prompt, None),
                    TurnInput::Command { name, arguments } => (arguments, Some(name)),
                };
                let request = AgentRequest {
                    directory,
                    prompt,
                    model: choice.model(),
                    command,
                    permissions,
                };
                session.run(&request, &|event| send(Message::Event(event)))
            })();
            if let Err(error) = result {
                send(failure(error));
            }
        };
        // A turn blocks on the agent, so it has a thread of its own. Under
        // test it runs on the test scheduler instead, which fails a test
        // when another thread wakes it, as this one's messages would.
        #[cfg(not(test))]
        std::thread::spawn(turn);
        #[cfg(test)]
        cx.background_executor()
            .spawn(async move { turn() })
            .detach();
        let task = cx.spawn_in(window, async move |this, cx| {
            while let Some(message) = messages.next().await {
                let alive = this
                    .update_in(cx, |this, window, cx| {
                        this.on_agent_message(id, message, window, cx)
                    })
                    .unwrap_or(false);
                if !alive {
                    break;
                }
            }
        });
        if let Some(run) = self.chat.as_mut() {
            run.bubble = bubble;
            run._task = Some(task);
        }
        if let (Some(action), Some(pane)) = (map_action, self.map.as_ref()) {
            pane.view
                .update(cx, |view, cx| view.working_on_action(action, cx));
        }
        cx.notify();
    }

    /// Send `text` to the agent as the user's reply, between turns.
    pub(in crate::workspace) fn send_to_agent(
        &mut self,
        text: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(run) = self.chat.as_mut() else {
            return;
        };
        let id = run.id;
        let Some(agent) = run.agent.as_mut().filter(|agent| agent.finished) else {
            return;
        };
        agent.entries.push(ChatEntry::User(text.clone()));
        agent.scroll.scroll_to_bottom();
        self.start_turn(id, TurnInput::Prompt(text), window, cx);
    }

    /// Send to Agent (⇧⌘L, or the code's right-click menu): the selection,
    /// or the cursor's line, goes into the conversation's reply box as a
    /// reference to write about and send. With no conversation open, a new
    /// one opens empty, waiting for the message.
    pub(in crate::workspace) fn send_selection_to_agent(
        &mut self,
        _: &SendSelectionToAgent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.home || self.map_showing().is_some() {
            return;
        }
        self.close_palette(window, cx);
        if self
            .chat
            .as_ref()
            .and_then(|run| run.agent.as_ref())
            .is_none()
        {
            let Some(id) = self.start_agent_run(window, cx) else {
                return;
            };
            let directory = self.agent_project(cx).expect("start_agent_run checked");
            let mut agent = self.new_agent_run(None, String::new(), directory, window, cx);
            agent.entries.clear();
            if let Some(run) = self.chat.as_mut().filter(|run| run.id == id) {
                run.agent = Some(agent);
            }
            let anchor = self.agent_anchor(window);
            if let Some(run) = self.chat.as_mut() {
                run.anchor = anchor;
            }
        }
        let Some(directory) = self
            .chat
            .as_ref()
            .and_then(|run| run.agent.as_ref())
            .map(|agent| agent.directory.clone())
        else {
            return;
        };
        let reference = self.selection_reference(&directory, cx);
        let Some(agent) = self.chat.as_mut().and_then(|run| run.agent.as_mut()) else {
            return;
        };
        let id = SharedString::from(format!("code-{}", agent.references.len()));
        let name = reference.name();
        let token = InlineToken::new(id.clone(), format!("@{name}")).with_label(name);
        agent.references.push((id, reference));
        let input = agent.input.clone();
        // Its reply box shows once the agent's turn is over.
        let waiting = agent.finished;
        input.update(cx, |input, cx| {
            let before = input.value()[..input.cursor()].to_string();
            if before
                .chars()
                .next_back()
                .is_some_and(|c| !c.is_whitespace())
            {
                input.insert(" ", window, cx);
            }
            if input.replace_with_token(token, window, cx).is_ok() {
                input.insert(" ", window, cx);
            }
        });
        if waiting {
            input.update(cx, |input, cx| input.focus(window, cx));
        } else {
            self.show_note(
                "It's in the reply box, for when the agent's done.".into(),
                window,
                cx,
            );
        }
        cx.notify();
    }

    /// The selection, or the cursor's line, as code sent to the agent, its
    /// file named from the project `directory`.
    fn selection_reference(&self, directory: &Path, cx: &App) -> CodeReference {
        let text = self.editor().text(cx);
        let selection = self.editor().selection(cx);
        let range = if selection.is_empty() {
            super::super::commands::line_at(&text, selection.start)
        } else {
            selection
        };
        let line = |offset: usize| 1 + text[..offset].matches('\n').count();
        let file = match self.document().path.as_deref() {
            Some(path) => path
                .strip_prefix(directory)
                .unwrap_or(path)
                .to_string_lossy()
                .into_owned(),
            None => "untitled".into(),
        };
        CodeReference {
            file,
            language: self.editor().language(cx),
            lines: (line(range.start), line(range.end.max(range.start + 1) - 1)),
            code: text[range].to_string(),
        }
    }

    /// Enter in the reply box: the agent takes the reply as its next turn.
    pub(super) fn reply_to_agent(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(run) = self.chat.as_mut() else {
            return;
        };
        let id = run.id;
        let Some(agent) = run.agent.as_mut().filter(|agent| agent.finished) else {
            return;
        };
        let text = agent.input.read(cx).value().trim().to_string();
        if text.is_empty() {
            // Signed in in the terminal: Enter tries again.
            if agent.sign_in.take().is_some()
                && let Some(input) = agent.last_input.clone()
            {
                agent.entries.push(ChatEntry::Note("Trying again.".into()));
                self.focus_agent_or_editor(window, cx);
                self.start_turn(id, input, window, cx);
            }
            return;
        }
        agent.sign_in = None;
        let references = referred_to(&agent.input.read(cx).content(), &agent.references);
        agent.references.clear();
        // Opened empty by sending code over: what's asked first names it.
        let first = agent.session.is_none() && agent.entries.is_empty();
        if first {
            agent.title = text.clone();
        }
        agent
            .input
            .update(cx, |input, cx| input.set_value("", window, cx));
        agent.entries.push(ChatEntry::User(text.clone()));
        agent.scroll.scroll_to_bottom();
        // Back to the code, where the agent's edits are reviewed, or the
        // Map, where a Map action's answer is.
        self.focus_code(window, cx);
        let input = self.slash_command(&text, cx).unwrap_or_else(|| {
            TurnInput::Prompt(jig_ai::agent::message(&text, &references, first))
        });
        self.start_turn(id, input, window, cx);
    }

    /// The sign-in option clicked: open the agent's login in a terminal, or
    /// ask the agent to sign in by itself and try again once it has.
    pub(super) fn sign_in_with(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(run) = self.chat.as_mut() else {
            return;
        };
        let id = run.id;
        let Some(agent) = run.agent.as_mut() else {
            return;
        };
        let Some(pending) = agent.sign_in.as_mut() else {
            return;
        };
        let Some(method) = pending.needed.methods.get(index).cloned() else {
            return;
        };
        match &method.terminal {
            Some(command) => match jig_agent::agent::open_in_terminal(command) {
                Ok(()) => {
                    pending.opened = true;
                    let input = agent.input.clone();
                    input.update(cx, |input, cx| input.focus(window, cx));
                }
                Err(error) => agent.entries.push(ChatEntry::Error(format!("{error:#}"))),
            },
            None => {
                agent.sign_in = None;
                agent
                    .entries
                    .push(ChatEntry::Note(format!("Signing in with {}…", method.name)));
                if let Some(input) = agent.last_input.clone() {
                    self.authenticate_then(id, method.id, input, window, cx);
                }
            }
        }
        cx.notify();
    }

    /// Ask the agent to sign in with `method` (it may open the browser),
    /// then send `input` again.
    fn authenticate_then(
        &mut self,
        id: u64,
        method: String,
        input: TurnInput,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let pick = self.agent_run(id).and_then(|agent| agent.pick.clone());
        let choice = jig_agent::agent::choice_for(pick.as_ref(), cx);
        let task = cx.background_executor().spawn(async move {
            let backend = jig_agent::agent::connect(&choice.map_err(anyhow::Error::msg)?)?;
            if let jig_ai::agent::harness::Backend::Acp(connection) = backend {
                connection.authenticate(&method)?;
            }
            anyhow::Ok(())
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            this.update_in(cx, |this, window, cx| match result {
                Ok(()) => this.start_turn(id, input, window, cx),
                Err(error) => {
                    this.on_agent_message(id, failure(error), window, cx);
                }
            })
            .ok();
        })
        .detach();
    }

    /// A reply like "/review main" runs that OpenCode command, when there
    /// is one by that name.
    fn slash_command(&self, text: &str, cx: &App) -> Option<TurnInput> {
        let typed = text.strip_prefix('/')?;
        let (name, arguments) = typed.split_once(' ').unwrap_or((typed, ""));
        self.catalog
            .read(cx)
            .catalog()?
            .commands
            .iter()
            .any(|command| command.name == name)
            .then(|| TurnInput::Command {
                name: name.to_string(),
                arguments: arguments.trim().to_string(),
            })
    }
}

/// The code a reply's tokens name, each once, in the order they're named.
pub(super) fn referred_to(
    content: &InputContent,
    references: &[(SharedString, CodeReference)],
) -> Vec<CodeReference> {
    let mut named: Vec<&SharedString> = Vec::new();
    content
        .tokens()
        .iter()
        .map(|span| span.token().id())
        .filter_map(|id| {
            if named.contains(&id) {
                return None;
            }
            named.push(id);
            references
                .iter()
                .find(|(known, _)| known == id)
                .map(|(_, reference)| reference.clone())
        })
        .collect()
}
