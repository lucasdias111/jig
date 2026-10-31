//! Starting, picking up and closing conversations, and loading the earlier ones.

use super::*;

impl Workspace {
    pub(in crate::workspace) fn run_agent(
        &mut self,
        id: u64,
        name: Option<String>,
        request: jig_ai::agent::Prompt,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(directory) = self.agent_project(cx) else {
            self.chat = None;
            return self.show_note(NO_PROJECT.into(), window, cx);
        };
        // What the user asked, as the conversation's first line.
        let asked = name
            .clone()
            .unwrap_or_else(|| request.instruction.trim().to_string());
        let agent = self.new_agent_run(name, asked, directory, window, cx);
        let run = self
            .chat
            .as_mut()
            .expect("start_conversation set up the conversation");
        run.agent = Some(agent);
        let anchor = self.agent_anchor(window);
        if let Some(run) = self.chat.as_mut() {
            run.anchor = anchor;
        }
        let prompt = jig_ai::agent::prompt(&request);
        self.start_turn(id, TurnInput::Prompt(prompt), window, cx);
    }

    /// Start a conversation with an OpenCode command or skill, e.g.
    /// "/review main".
    pub(in crate::workspace) fn run_agent_command(
        &mut self,
        name: String,
        arguments: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(id) = self.start_agent_run(window, cx) else {
            return;
        };
        let asked = format!("/{name} {arguments}").trim().to_string();
        let directory = self.agent_project(cx).expect("start_agent_run checked");
        let agent = self.new_agent_run(Some(format!("/{name}")), asked, directory, window, cx);
        if let Some(run) = self.chat.as_mut() {
            run.agent = Some(agent);
        }
        self.start_turn(id, TurnInput::Command { name, arguments }, window, cx);
    }

    /// Pick up the earlier conversation `session_id` where it left off.
    pub(in crate::workspace) fn resume_agent(
        &mut self,
        session_id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(id) = self.start_agent_run(window, cx) else {
            return;
        };
        let title = self
            .catalog
            .read(cx)
            .catalog()
            .and_then(|catalog| {
                catalog
                    .conversations
                    .iter()
                    .find(|conversation| conversation.id == session_id)
            })
            .map(conversation_title)
            .unwrap_or_else(|| "Earlier conversation".into());
        let directory = self.agent_project(cx).expect("start_agent_run checked");
        let mut agent = self.new_agent_run(Some(title.clone()), title, directory, window, cx);
        agent.entries.clear();
        agent.loading = true;
        let directory = agent.directory.clone();
        let choice = jig_agent::agent::choice_for(agent.pick.as_ref(), cx);
        if let Some(run) = self.chat.as_mut() {
            run.agent = Some(agent);
        }

        let (sender, mut messages) = futures::channel::mpsc::unbounded();
        let resume = move || {
            let send = |message| {
                let _ = sender.unbounded_send(message);
            };
            let result = (|| {
                let choice = choice.map_err(anyhow::Error::msg)?;
                let backend = jig_agent::agent::connect(&choice)?;
                let session = backend.open(&directory, &session_id)?;
                let turns = session.history()?;
                if let Some(model) = choice.acp_model() {
                    session.use_model(model)?;
                }
                if let Some(models) = backend.models() {
                    send(Message::Models(choice.key().to_string(), models));
                }
                anyhow::Ok(Message::Resumed(session, turns))
            })();
            send(result.unwrap_or_else(failure));
        };
        // On a thread of its own, as a turn is (see `start_turn`).
        #[cfg(not(test))]
        std::thread::spawn(resume);
        #[cfg(test)]
        cx.background_executor()
            .spawn(async move { resume() })
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
            run._task = Some(task);
        }
        cx.notify();
    }

    /// Set up an empty run for a conversation at the cursor. `None`, with
    /// the reason shown, when the file isn't saved anywhere yet.
    pub(super) fn start_agent_run(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<u64> {
        if self.mode == super::super::mode::Mode::Agent && self.agent_busy() {
            self.show_note(
                "Finish or stop the current conversation first.".into(),
                window,
                cx,
            );
            return None;
        }
        self.close_chat(window, cx);
        self.next_run_id += 1;
        let id = self.next_run_id;
        let anchor = self.agent_anchor(window);
        self.chat = Some(CommandRun {
            id,
            bubble: Bubble::Message(String::new()),
            anchor,
            preview: None,
            agent: None,
            _task: None,
        });
        if self.agent_project(cx).is_none() {
            self.chat = None;
            self.show_note(NO_PROJECT.into(), window, cx);
            return None;
        }
        Some(id)
    }

    /// Close the conversation on screen. An edit of its still under review
    /// is undone first: nothing reaches the disk unreviewed. The
    /// conversation stays with the agent, in the history.
    pub(in crate::workspace) fn close_chat(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.settle_agent_edit(false, Some(window), cx);
        if self
            .chat
            .as_ref()
            .and_then(|run| run.agent.as_ref())
            .is_some_and(|agent| agent.map_action.is_some() && !agent.finished)
        {
            self.end_map_action(cx);
        }
        if self.chat.take().is_some() {
            self.editor().clear_highlights(cx);
            self.sync_map_drawings(cx);
            cx.notify();
        }
    }

    /// ⌥⌘K, or the conversation's history button: the ⌘K input, which
    /// lists the earlier conversations.
    pub(in crate::workspace) fn open_agent_conversations(
        &mut self,
        _: &OpenAgentConversations,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.palette.is_none() {
            self.open_command(&OpenCommand, window, cx);
        }
    }

    /// Fill the ⌘K input's lists: what's known at once, then what the agent
    /// says now. Without a palette, just learn the commands, so a
    /// reply can run one.
    pub(in crate::workspace) fn load_agent_catalog(
        &mut self,
        palette: Option<Entity<CommandPalette>>,
        cx: &mut Context<Self>,
    ) {
        let Some(directory) = self.agent_project(cx) else {
            return;
        };
        let agent = self.picked_agent(cx).key.clone();
        if let Some(catalog) = self
            .catalog
            .read(cx)
            .for_project(&directory, &agent)
            .cloned()
            && let Some(palette) = &palette
        {
            show_catalog(&catalog, palette, cx);
        }
        let choice = jig_agent::agent::choice_for(self.agent.pick.as_ref(), cx);
        self.catalog.update(cx, |catalog, cx| {
            catalog.load(directory, agent, choice, palette, cx)
        });
        cx.notify();
    }

    /// A load of the catalog finished: keep it, and fill the ⌘K input that
    /// asked, unless the project or agent has changed since.
    pub(in crate::workspace) fn on_catalog_loaded(
        &mut self,
        catalog: Entity<ConversationCatalog>,
        loaded: &CatalogLoaded,
        cx: &mut Context<Self>,
    ) {
        if self.agent_project(cx).as_ref() != Some(&loaded.directory)
            || self.picked_agent(cx).key != loaded.agent
        {
            cx.notify();
            return;
        }
        match &loaded.result {
            Err(error) => {
                catalog.update(cx, |catalog, cx| catalog.failed(error.clone(), cx));
            }
            Ok(found) => {
                if let Some(palette) = &loaded.palette {
                    show_catalog(found, palette, cx);
                }
                let found = found.clone();
                catalog.update(cx, |catalog, cx| catalog.keep(found, cx));
            }
        }
        cx.notify();
    }

    /// A conversation that starts with the user asking `asked`, in the
    /// project around `path`. The agent isn't contacted yet.
    pub(in crate::workspace) fn new_agent_run(
        &mut self,
        name: Option<String>,
        asked: String,
        directory: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AgentRun {
        let input = cx.new(|cx| {
            TextareaState::new(window, cx)
                .placeholder("Reply to the agent…")
                .auto_grow(1, 8)
                .soft_wrap(true)
        });
        let context = self.context_names();
        let project = directory
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| directory.display().to_string());
        AgentRun {
            session: None,
            pending: None,
            queue: VecDeque::new(),
            loading: false,
            last_input: None,
            pick: Some(
                self.agent
                    .pick
                    .clone()
                    .unwrap_or_else(|| jig_agent::agent::Pick::from_settings(cx)),
            ),
            models_open: false,
            sign_in: None,
            always_allowed: Vec::new(),
            follow_up: None,
            accept_edits: false,
            title: asked.clone(),
            touched: Vec::new(),
            step: LiveStep::default(),
            label: name.unwrap_or_else(|| self.picked_agent(cx).name.clone()),
            started: std::time::Instant::now(),
            context,
            finished: true,
            directory,
            project,
            entries: vec![ChatEntry::User(asked)],
            input,
            page_focus: cx.focus_handle(),
            references: Vec::new(),
            scroll: ScrollHandle::new(),
            frame: match self.agent.dock {
                Some(width) => ChatFrame {
                    width,
                    pinned: true,
                    ..ChatFrame::default()
                },
                None => ChatFrame::default(),
            },
            grab: None,
            resize: None,
            moved: false,
            map_action: None,
            proposed: None,
            drawings: Vec::new(),
        }
    }
}

/// Hand the catalog to the palette, if it's still open.
fn show_catalog(catalog: &AgentCatalog, palette: &Entity<CommandPalette>, cx: &mut App) {
    let now = SystemTime::now();
    let conversations = catalog
        .conversations
        .iter()
        .map(|conversation| PastConversation {
            id: conversation.id.clone(),
            title: conversation_title(conversation),
            when: ago(now, conversation.updated),
        })
        .collect();
    let commands = catalog.commands.clone();
    palette.update(cx, |palette, cx| {
        palette.set_agent_catalog(conversations, commands, cx)
    });
}
