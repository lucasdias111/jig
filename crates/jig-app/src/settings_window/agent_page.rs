//! Agent: which agent Agent mode runs (OpenCode, Claude Code, Codex, Gemini
//! CLI or one from `agents.toml`), and what it may do besides reading the
//! project and proposing edits, each asked about every time, allowed, or
//! turned off. Kept under `[agent]` in `settings.toml` and applied from its
//! next turn.

use std::sync::Arc;

use gpui_kit::component::Disableable as _;
use gpui_kit::*;
use jig_ai::agent::{Access, Permissions};

use super::ui::{self, Row, Section};
use super::{Fetch, SettingsWindow, send_to_workspace};
use crate::agents::{self, Agent};
use crate::settings;
use crate::workspace::EditAgents;

impl SettingsWindow {
    pub(super) fn agent_sections(&self, cx: &Context<Self>) -> Vec<Section> {
        vec![self.agents_section(cx), permissions_section()]
    }

    /// Which agent runs, with where Jig found it or how to get it.
    fn agents_section(&self, cx: &Context<Self>) -> Section {
        let registry = agents::registry();
        let options = registry
            .agents
            .iter()
            .map(|agent| (agent.key.clone().into(), agent.name.clone().into()))
            .collect();
        let chosen = agents::chosen(cx);
        let (status, ready) = agents::status(&chosen);
        let mut section = Section::new().row(
            Row::new("agent", "Agent")
                .description(format!(
                    "{} {}.",
                    chosen.about,
                    status.trim_end_matches('.')
                ))
                .warning(!ready)
                .keywords(["agent", "claude", "codex", "gemini", "opencode", "acp"])
                .control(ui::dropdown(
                    "agent",
                    options,
                    |cx| agents::chosen(cx).key.clone().into(),
                    |key, cx| settings::update(cx, |s| s.agent.using = key.to_string()),
                )),
        );
        if chosen.installable() && (!ready || self.installs.contains_key(&install_key(&chosen))) {
            section = section.row(self.install_agent_row(chosen, cx));
        }
        let error = registry.error.clone();
        section.row(
            Row::new("agents-file", "Agents file")
                .description_opt(error.clone().or(Some(
                    "Add any agent that speaks the Agent Client Protocol in agents.toml.".into(),
                )))
                .warning(error.is_some())
                .keywords(["agent", "add", "file", "toml", "acp"])
                .control(|_, _| {
                    ui::button("edit-agents", "Edit…")
                        .on_click(|_, _, cx| send_to_workspace(Box::new(EditAgents), cx))
                }),
        )
    }

    /// Install the chosen agent, and how that went.
    fn install_agent_row(&self, agent: Arc<Agent>, cx: &Context<Self>) -> Row {
        let this = cx.entity().downgrade();
        let fetch = self.installs.get(&install_key(&agent));
        let installing = matches!(fetch, Some(Fetch::Running));
        let (description, ok) = match fetch {
            None => (
                format!("Into Jig's agents folder, with {}.", agent.name),
                true,
            ),
            Some(Fetch::Running) => ("Installing…".to_string(), true),
            Some(Fetch::Done(())) => ("Installed.".to_string(), true),
            Some(Fetch::Failed(error)) => (error.to_string(), false),
        };
        let id = format!("install-agent-{}", agent.key);
        Row::new(id.clone(), format!("Install {}", agent.name))
            .description(description)
            .warning(!ok)
            .keywords(["install", "agent"])
            .control(move |_, _| {
                let this = this.clone();
                let agent = agent.clone();
                ui::button(id.clone(), "Install")
                    .disabled(installing)
                    .on_click(move |_, _, cx| {
                        let agent = agent.clone();
                        this.update(cx, |this, cx| this.install_agent(agent, cx))
                            .ok();
                    })
            })
    }

    fn install_agent(&mut self, agent: Arc<Agent>, cx: &mut Context<Self>) {
        let key = install_key(&agent);
        if matches!(self.installs.get(&key), Some(Fetch::Running)) {
            return;
        }
        self.installs.insert(key.clone(), Fetch::Running);
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { agents::install(&agent) })
                .await;
            this.update(cx, |this, cx| {
                let fetch = match result {
                    Ok(()) => Fetch::Done(()),
                    Err(error) => Fetch::Failed(format!("{error:#}").into()),
                };
                this.installs.insert(key, fetch);
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }
}

fn permissions_section() -> Section {
    let access_row = |id: &'static str,
                      label: &'static str,
                      description: &'static str,
                      keywords: &'static [&'static str],
                      get: fn(&Permissions) -> Access,
                      set: fn(&mut Permissions, Access)| {
        let options = Access::ALL
            .into_iter()
            .map(|access| (access.key().into(), access_label(access).into()))
            .collect();
        Row::new(id, label)
            .description(description)
            .keywords(keywords.iter().copied().chain(["agent", "permission"]))
            .control(ui::dropdown(
                id,
                options,
                move |cx| get(&settings::get(cx).agent.permissions).key().into(),
                move |key, cx| {
                    settings::update(cx, |s| {
                        set(&mut s.agent.permissions, Access::from_key(&key))
                    })
                },
            ))
    };
    Section::titled("Permissions")
        .footer(
            "Edits always wait for your review, and nothing outside the project is \
             touched. Asked, Enter allows once and Esc declines. Changes apply from the \
             agent's next turn.",
        )
        .row(access_row(
            "shell",
            "Shell commands",
            "Commands it runs in the project, like cargo test.",
            &["shell", "bash", "command", "terminal"],
            |p| p.shell,
            |p, access| p.shell = access,
        ))
        .row(access_row(
            "web-fetch",
            "Web pages",
            "Pages it fetches to read, like documentation.",
            &["web", "fetch", "url", "internet"],
            |p| p.web_fetch,
            |p, access| p.web_fetch = access,
        ))
        .row(access_row(
            "web-search",
            "Web search",
            "Searches it runs to find something out.",
            &["web", "search", "internet"],
            |p| p.web_search,
            |p, access| p.web_search = access,
        ))
        .row(access_row(
            "subagents",
            "Subagents",
            "Agents it starts for part of the task. Their edits wait for review too.",
            &["subagent", "task"],
            |p| p.subagents,
            |p, access| p.subagents = access,
        ))
        .row(
            Row::new("questions", "Questions with choices")
                .description("OpenCode asks with options to pick from. Off, it asks in its reply.")
                .keywords(["question", "ask", "agent", "permission"])
                .control(ui::switch(
                    "questions",
                    |cx| settings::get(cx).agent.permissions.questions,
                    |on, cx| settings::update(cx, |s| s.agent.permissions.questions = on),
                )),
        )
}

fn access_label(access: Access) -> &'static str {
    match access {
        Access::Ask => "Ask each time",
        Access::Allow => "Allow",
        Access::Deny => "Don't allow",
    }
}

/// Where an agent's install shows in `installs`, apart from debuggers'.
fn install_key(agent: &Agent) -> String {
    format!("agent:{}", agent.key)
}
