//! Settings > Agent: which agent Agent mode runs (OpenCode, Claude Code,
//! Codex, Gemini CLI or one from `agents.toml`), and what it may do besides
//! reading the project and proposing edits, each asked about every time,
//! allowed, or turned off. Kept under `[agent]` in `settings.toml` and
//! applied from its next turn.

use std::sync::Arc;

use gpui_kit::component::button::Button;
use gpui_kit::component::setting::{SettingField, SettingGroup, SettingItem, SettingPage};
use gpui_kit::component::{ActiveTheme as _, Disableable as _, Icon, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use jig_ai::agent::{Access, Permissions};

use super::{Fetch, SettingsWindow, send_to_workspace};
use crate::agents::{self, Agent};
use crate::settings;
use crate::workspace::EditAgents;

impl SettingsWindow {
    /// Which agent runs, with where Jig found it or how to get it.
    fn agents_group(&self, cx: &Context<Self>) -> SettingGroup {
        let registry = agents::registry();
        let options = registry
            .agents
            .iter()
            .map(|agent| (agent.key.clone().into(), agent.name.clone().into()))
            .collect();
        let chosen = agents::chosen(cx);
        let (status, ready) = agents::status(&chosen);
        let mut group = SettingGroup::new()
            .title("Agent")
            .description(
                "What runs in Agent mode. Any agent that speaks the Agent Client \
                 Protocol can be added in agents.toml.",
            )
            .item(
                SettingItem::new(
                    "Agent",
                    SettingField::dropdown(
                        options,
                        |cx| agents::chosen(cx).key.clone().into(),
                        |key: SharedString, cx| {
                            settings::update(cx, |s| s.agent.using = key.to_string())
                        },
                    )
                    .default_value(agents::DEFAULT),
                )
                .description(format!(
                    "{} {}.",
                    chosen.about,
                    status.trim_end_matches('.')
                ))
                .keywords(["agent", "claude", "codex", "gemini", "opencode", "acp"]),
            );
        if chosen.installable() && (!ready || self.installs.contains_key(&install_key(&chosen))) {
            group = group.item(self.install_agent_item(chosen, cx));
        }
        let error = registry.error.clone();
        group.item(
            SettingItem::render(move |_, _, cx| {
                v_flex()
                    .gap_2()
                    .when_some(error.clone(), |this, error| {
                        this.child(div().text_sm().text_color(cx.theme().danger).child(error))
                    })
                    .child(
                        h_flex().child(
                            Button::new("edit-agents")
                                .label("Edit Agents File")
                                .small()
                                .outline()
                                .on_click(|_, _, cx| send_to_workspace(Box::new(EditAgents), cx)),
                        ),
                    )
            })
            .keywords(["agent", "add", "file", "toml", "acp"]),
        )
    }

    /// Install the chosen agent, and how that went.
    fn install_agent_item(&self, agent: Arc<Agent>, cx: &Context<Self>) -> SettingItem {
        let this = cx.entity().downgrade();
        let fetch = self.installs.get(&install_key(&agent));
        let installing = matches!(fetch, Some(Fetch::Running));
        let report = fetch.map(|fetch| match fetch {
            Fetch::Running => (SharedString::from("Installing…"), true),
            Fetch::Done(()) => ("Installed.".into(), true),
            Fetch::Failed(error) => (error.clone(), false),
        });
        let id = SharedString::from(format!("install-agent-{}", agent.key));
        let label = format!("Install {}", agent.name);
        SettingItem::render(move |_, _, cx| {
            let this = this.clone();
            let agent = agent.clone();
            v_flex()
                .gap_1()
                .child(
                    h_flex().child(
                        Button::new(id.clone())
                            .label(label.clone())
                            .small()
                            .outline()
                            .disabled(installing)
                            .on_click(move |_, _, cx| {
                                let agent = agent.clone();
                                this.update(cx, |this, cx| this.install_agent(agent, cx))
                                    .ok();
                            }),
                    ),
                )
                .when_some(report.clone(), |this, (text, ok)| {
                    this.child(
                        div()
                            .text_sm()
                            .text_color(if ok {
                                cx.theme().muted_foreground
                            } else {
                                cx.theme().danger
                            })
                            .child(text),
                    )
                })
        })
        .keywords(["install", "agent"])
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

    pub(super) fn agent_page(&self, cx: &Context<Self>) -> SettingPage {
        let access_item = |label: &'static str,
                           description: &'static str,
                           keywords: &'static [&'static str],
                           get: fn(&Permissions) -> Access,
                           set: fn(&mut Permissions, Access)| {
            let options = Access::ALL
                .into_iter()
                .map(|access| (access.key().into(), access_label(access).into()))
                .collect();
            SettingItem::new(
                label,
                SettingField::dropdown(
                    options,
                    move |cx| get(&settings::get(cx).agent.permissions).key().into(),
                    move |key: SharedString, cx| {
                        settings::update(cx, |s| {
                            set(&mut s.agent.permissions, Access::from_key(&key))
                        })
                    },
                )
                .default_value(get(&Permissions::default()).key()),
            )
            .description(description)
            .keywords(keywords.iter().copied().chain(["agent", "permission"]))
        };

        SettingPage::new("Agent")
            .icon(Icon::default().data(jig_commands::surface::AGENT_ICON))
            .group(self.agents_group(cx))
            .group(
                SettingGroup::new()
                    .title("Permissions")
                    .description(
                        "What the agent may do in Agent mode. Every edit it asks about \
                         waits for your review, and OpenCode never touches files outside \
                         the project. Changes apply from its next turn.",
                    )
                    .item(access_item(
                        "Shell commands",
                        "Commands it runs in the project, like cargo test.",
                        &["shell", "bash", "command", "terminal"],
                        |p| p.shell,
                        |p, access| p.shell = access,
                    ))
                    .item(access_item(
                        "Web pages",
                        "Pages it fetches to read, like documentation.",
                        &["web", "fetch", "url", "internet"],
                        |p| p.web_fetch,
                        |p, access| p.web_fetch = access,
                    ))
                    .item(access_item(
                        "Web search",
                        "Searches it runs to find something out.",
                        &["web", "search", "internet"],
                        |p| p.web_search,
                        |p, access| p.web_search = access,
                    ))
                    .item(access_item(
                        "Subagents",
                        "Agents it starts for part of the task. Their edits wait for \
                         review like its own.",
                        &["subagent", "task"],
                        |p| p.subagents,
                        |p, access| p.subagents = access,
                    ))
                    .item(
                        SettingItem::new(
                            "Questions with choices",
                            SettingField::switch(
                                |cx| settings::get(cx).agent.permissions.questions,
                                |on, cx| {
                                    settings::update(cx, |s| s.agent.permissions.questions = on)
                                },
                            )
                            .default_value(Permissions::default().questions),
                        )
                        .description(
                            "OpenCode can ask with options to pick from. Off, it asks in its \
                             reply.",
                        )
                        .keywords([
                            "question",
                            "ask",
                            "agent",
                            "permission",
                        ]),
                    )
                    .item(SettingItem::render(|_, _, cx| {
                        div()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child(
                                "Allowed, what it ran still shows in the conversation. \
                                 Asked, Enter allows it once and Esc turns it down.",
                            )
                    })),
            )
    }
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
