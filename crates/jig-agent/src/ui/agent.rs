//! The agent as the app picks it: from the settings and the window's pick,
//! read on the main thread, where the settings and keys are.

use std::sync::Arc;

use gpui_kit::App;

use crate::domain::agents::{Agent, Kind};
use crate::domain::choice::{Choice, Pick};
use crate::store::persist::agents::named;
use crate::store::session::stop;
use crate::ui::providers;

impl Pick {
    /// What the settings say: the agent picked there, on its model.
    pub fn from_settings(cx: &App) -> Self {
        let agent = chosen(cx);
        let model = match agent.kind {
            Kind::OpenCode => providers::config(cx)
                .ok()
                .and_then(|config| config.agent.clone().filter(|id| !id.is_empty())),
            Kind::Acp => jig_settings::get(cx).agent.models.get(&agent.key).cloned(),
        };
        Self {
            agent: agent.key.clone(),
            model,
        }
    }
}

/// The agent `pick` names, else the one Settings picked, ready to start.
pub fn choice_for(pick: Option<&Pick>, cx: &App) -> Result<Choice, String> {
    let pick = pick.cloned().unwrap_or_else(|| Pick::from_settings(cx));
    let agent = named(&pick.agent);
    match agent.kind {
        Kind::OpenCode => {
            providers::agent_target_for(pick.model.as_deref(), cx).map(Choice::OpenCode)
        }
        Kind::Acp => Ok(Choice::Acp {
            key: agent.key.clone(),
            name: agent.name.clone(),
            command: agent
                .command_line()
                .map_err(|missing| format!("{missing} Install it in Settings > Agent."))?,
            model: pick.model,
        }),
    }
}

/// Stop the servers on quit; a static is never dropped on its own.
pub fn init(cx: &mut App) {
    cx.on_app_quit(|_| {
        stop();
        async {}
    })
    .detach();
}

/// The agent Settings picked, or OpenCode.
pub fn chosen(cx: &App) -> Arc<Agent> {
    named(&jig_settings::get(cx).agent.using)
}
