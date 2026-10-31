//! Which agent and model a conversation runs on, and picking another.

use super::*;

impl Workspace {
    /// What ⌘K's model chip offers: each installed agent with its models
    /// (OpenCode's from the providers, the others' as they last said).
    pub(in crate::workspace) fn model_menu(&self, cx: &App) -> ModelMenu {
        let offered = jig_agent::providers::offered_by_provider(cx);
        let picked = self.picked_agent(cx);
        let pick = self
            .agent
            .pick
            .clone()
            .unwrap_or_else(|| jig_agent::agent::Pick::from_settings(cx));
        let mut agent = ModelMenu::default();
        for listed in jig_agent::agents::registry().agents.iter() {
            // Agents not installed would only fail; the chosen one shows anyway.
            if listed.key != picked.key && !jig_agent::agents::status(listed).1 {
                continue;
            }
            let own: Vec<(String, String)> = match listed.kind {
                jig_agent::agents::Kind::OpenCode => offered
                    .iter()
                    .map(|(id, provider, model)| (id.clone(), format!("{provider} · {model}")))
                    .collect(),
                jig_agent::agents::Kind::Acp => jig_agent::agents::known_models(&listed.key)
                    .map(|models| models.options)
                    .unwrap_or_default(),
            };
            if own.is_empty() {
                agent.choices.push(ModelChoice {
                    agent: Some(listed.key.clone()),
                    group: listed.name.clone(),
                    name: "Default model".into(),
                    model: None,
                    label: listed.name.clone(),
                });
            }
            for (model, name) in own {
                agent.choices.push(ModelChoice {
                    agent: Some(listed.key.clone()),
                    group: listed.name.clone(),
                    label: format!("{} · {name}", listed.name),
                    name,
                    model: Some(model),
                });
            }
        }
        agent.current = Some(position_or_push(
            &mut agent.choices,
            Some(&pick.agent),
            pick.model.as_deref(),
            &picked.name,
            pick.model.as_deref().unwrap_or("Default model"),
        ));
        agent
    }

    /// The palette's lists again, as the providers' models arrive.
    pub(in crate::workspace) fn refresh_palette_models(&self, cx: &mut Context<Self>) {
        let Some(palette) = self.palette.as_ref().map(|palette| palette.view.clone()) else {
            return;
        };
        let menu = self.model_menu(cx);
        palette.update(cx, |palette, cx| palette.set_models(menu, cx));
    }

    /// ⌘K's chip picked `choice` for the window: the agent and model new
    /// conversations run.
    pub(in crate::workspace) fn pick_model(
        &mut self,
        choice: ModelChoice,
        palette: Entity<CommandPalette>,
        cx: &mut Context<Self>,
    ) {
        let Some(agent) = choice.agent else {
            return;
        };
        let pick = jig_agent::agent::Pick {
            agent,
            model: choice.model,
        };
        // The settings' own choice needs no pick.
        self.agent.pick = (pick != jig_agent::agent::Pick::from_settings(cx)).then_some(pick);
        if self.mode == super::super::mode::Mode::Agent {
            let pick = self
                .agent
                .pick
                .clone()
                .unwrap_or_else(|| jig_agent::agent::Pick::from_settings(cx));
            let label = self.picked_agent(cx).name.clone();
            if let Some(agent) = self
                .chat
                .as_mut()
                .and_then(|run| run.agent.as_mut())
                .filter(|agent| {
                    agent.entries.is_empty() && agent.session.is_none() && agent.finished
                })
            {
                agent.pick = Some(pick);
                agent.label = label;
            }
        }
        // Its conversations and commands, in place of the last agent's.
        self.load_agent_catalog(Some(palette), cx);
        cx.notify();
    }

    /// What the open conversation can run on: its agent's models, each as
    /// the model value to use, and as its header lists them.
    pub(super) fn conversation_models(&self, cx: &App) -> (Vec<Option<String>>, ChatModels) {
        let Some(pick) = self
            .chat
            .as_ref()
            .and_then(|chat| chat.agent.as_ref())
            .and_then(|agent| agent.pick.clone())
        else {
            return (Vec::new(), ChatModels::default());
        };
        let open = self
            .chat
            .as_ref()
            .and_then(|chat| chat.agent.as_ref())
            .is_some_and(|agent| agent.models_open);
        let agent = jig_agent::agents::named(&pick.agent);
        let listed: Vec<(String, String)> = match agent.kind {
            jig_agent::agents::Kind::OpenCode => jig_agent::providers::offered(cx),
            jig_agent::agents::Kind::Acp => jig_agent::agents::known_models(&agent.key)
                .map(|models| models.options)
                .unwrap_or_default(),
        };
        let mut values: Vec<Option<String>> = listed
            .iter()
            .map(|(value, _)| Some(value.clone()))
            .collect();
        let mut labels: Vec<String> = listed.into_iter().map(|(_, label)| label).collect();
        // On its default, or on a model no longer listed: say so.
        let in_use = pick
            .model
            .clone()
            .or_else(|| jig_agent::agents::known_models(&agent.key).map(|models| models.current));
        let current = match values.iter().position(|value| *value == in_use) {
            Some(at) => at,
            None => {
                values.push(in_use.clone());
                labels.push(in_use.unwrap_or_else(|| "Default model".into()));
                labels.len() - 1
            }
        };
        (
            values,
            ChatModels {
                options: labels,
                current: Some(current),
                open,
            },
        )
    }

    pub(super) fn toggle_conversation_models(&mut self, cx: &mut Context<Self>) {
        if let Some(agent) = self.chat.as_mut().and_then(|chat| chat.agent.as_mut()) {
            agent.models_open = !agent.models_open;
            cx.notify();
        }
    }

    /// The header's model picked: the conversation goes on with it. An ACP
    /// agent is switched now; OpenCode takes it with the next message.
    pub(super) fn pick_conversation_model(&mut self, index: usize, cx: &mut Context<Self>) {
        let (values, models) = self.conversation_models(cx);
        let Some(model) = values.get(index).cloned() else {
            return;
        };
        let Some(agent) = self.chat.as_mut().and_then(|chat| chat.agent.as_mut()) else {
            return;
        };
        agent.models_open = false;
        let Some(pick) = agent.pick.as_mut() else {
            return;
        };
        if pick.model == model {
            return cx.notify();
        }
        pick.model = model.clone();
        let label = models.options.get(index).cloned().unwrap_or_default();
        agent
            .entries
            .push(ChatEntry::Note(format!("Now on {label}.")));
        agent.scroll.scroll_to_bottom();
        if let (Some(session), Some(model)) = (agent.session.clone(), model) {
            cx.background_executor()
                .spawn(async move {
                    let _ = session.use_model(&model);
                })
                .detach();
        }
        cx.notify();
    }

    /// The agent new conversations run: picked in ⌘K, else in Settings.
    pub(in crate::workspace) fn picked_agent(
        &self,
        cx: &App,
    ) -> std::sync::Arc<jig_agent::agents::Agent> {
        match &self.agent.pick {
            Some(pick) => jig_agent::agents::named(&pick.agent),
            None => jig_agent::agents::chosen(cx),
        }
    }

    /// Open `agents.toml`, starting it with a commented example.
    pub(in crate::workspace) fn edit_agents(
        &mut self,
        _: &EditAgents,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(path) = jig_agent::agents::user_path() else {
            return;
        };
        if !path.exists() {
            let created = path
                .parent()
                .map_or(Ok(()), std::fs::create_dir_all)
                .and_then(|()| std::fs::write(&path, jig_agent::agents::USER_TEMPLATE));
            if let Err(error) = created {
                self.show_error(
                    &format!("Couldn't create {}: {error}", path.display()),
                    window,
                    cx,
                );
                return;
            }
        }
        self.open_file(&path, window, cx);
    }
}

/// Where the choice for `agent` and `model` is in `choices`, adding it
/// (under `group`, as `name`) when it isn't listed, e.g. while models load.
fn position_or_push(
    choices: &mut Vec<ModelChoice>,
    agent: Option<&str>,
    model: Option<&str>,
    group: &str,
    name: &str,
) -> usize {
    let same_agent = |choice: &ModelChoice| choice.agent.as_deref() == agent;
    if let Some(at) = choices
        .iter()
        .position(|choice| same_agent(choice) && choice.model.as_deref() == model)
    {
        return at;
    }
    // An agent on its default: the first of its own.
    if model.is_none()
        && let Some(at) = choices.iter().position(same_agent)
    {
        return at;
    }
    choices.push(ModelChoice {
        agent: agent.map(str::to_string),
        group: group.to_string(),
        name: name.to_string(),
        model: model.map(str::to_string),
        label: format!("{group} · {name}"),
    });
    choices.len() - 1
}
