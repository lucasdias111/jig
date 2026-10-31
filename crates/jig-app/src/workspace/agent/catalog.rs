//! The agent's earlier conversations and commands for the project, which
//! the ⌘K input and Agent mode list, and a reply's `/command` runs. An
//! entity of its own: it loads them in the background and keeps the last
//! answer; the workspace, which knows the project and agent now current,
//! decides whether a load that finished still applies.

use std::path::{Path, PathBuf};

use gpui_kit::*;
use jig_agent::agent::Choice;
use jig_ai::agent::Conversation;
use jig_commands::{AgentCommandInfo, CommandPalette};

#[derive(Default)]
pub(in crate::workspace) struct ConversationCatalog {
    /// What was last loaded.
    catalog: Option<AgentCatalog>,
    loading: bool,
    error: Option<String>,
    /// Which load is the latest, so an older one finishing late is
    /// dropped.
    generation: u64,
}

/// The earlier conversations and OpenCode's commands for one project, kept
/// so the palette can list them at once the next time.
#[derive(Clone)]
pub(in crate::workspace) struct AgentCatalog {
    /// The agent's key, as Settings or ⌘K picked it.
    pub(in crate::workspace) agent: String,
    pub(in crate::workspace) directory: PathBuf,
    pub(in crate::workspace) conversations: Vec<Conversation>,
    pub(in crate::workspace) commands: Vec<AgentCommandInfo>,
}

/// The latest load finished, for agent `agent` in `directory`. `palette`
/// is the ⌘K input that asked, if one did.
pub(in crate::workspace) struct CatalogLoaded {
    pub(in crate::workspace) agent: String,
    pub(in crate::workspace) directory: PathBuf,
    pub(in crate::workspace) palette: Option<Entity<CommandPalette>>,
    pub(in crate::workspace) result: Result<AgentCatalog, String>,
}

impl EventEmitter<CatalogLoaded> for ConversationCatalog {}

impl ConversationCatalog {
    /// What was last loaded, whichever project and agent it's for.
    pub(in crate::workspace) fn catalog(&self) -> Option<&AgentCatalog> {
        self.catalog.as_ref()
    }

    /// What was last loaded, if it's for agent `agent` in `directory`.
    pub(in crate::workspace) fn for_project(
        &self,
        directory: &Path,
        agent: &str,
    ) -> Option<&AgentCatalog> {
        self.catalog
            .as_ref()
            .filter(|catalog| catalog.directory == directory && catalog.agent == agent)
    }

    pub(in crate::workspace) fn loading(&self) -> bool {
        self.loading
    }

    /// Why the last load failed, until the next starts.
    pub(in crate::workspace) fn error(&self) -> Option<&String> {
        self.error.as_ref()
    }

    /// Ask agent `agent`, reached through `choice`, for its conversations
    /// and commands in `directory`, in the background. `palette` is the
    /// ⌘K input to fill once they're in.
    pub(in crate::workspace) fn load(
        &mut self,
        directory: PathBuf,
        agent: String,
        choice: Result<Choice, String>,
        palette: Option<Entity<CommandPalette>>,
        cx: &mut Context<Self>,
    ) {
        self.generation += 1;
        let generation = self.generation;
        self.loading = true;
        self.error = None;
        cx.notify();
        let task = cx.background_executor().spawn({
            let directory = directory.clone();
            async move {
                let choice = choice.map_err(anyhow::Error::msg)?;
                let backend = jig_agent::agent::connect(&choice)?;
                let conversations = backend.conversations(&directory)?;
                let commands = backend.commands(&directory).unwrap_or_default();
                anyhow::Ok((conversations, commands))
            }
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            this.update(cx, |this, cx| {
                if generation != this.generation {
                    return;
                }
                this.loading = false;
                cx.notify();
                let result = result
                    .map(|(conversations, commands)| AgentCatalog {
                        agent: agent.clone(),
                        directory: directory.clone(),
                        conversations,
                        commands: commands
                            .into_iter()
                            .map(|command| AgentCommandInfo {
                                name: command.name,
                                description: command.description,
                                skill: command.skill,
                            })
                            .collect(),
                    })
                    .map_err(|error| format!("Couldn't load conversations: {error:#}"));
                cx.emit(CatalogLoaded {
                    agent,
                    directory,
                    palette,
                    result,
                });
            })
            .ok();
        })
        .detach();
    }

    /// Keep `catalog`, the answer to the latest load.
    pub(in crate::workspace) fn keep(&mut self, catalog: AgentCatalog, cx: &mut Context<Self>) {
        self.catalog = Some(catalog);
        cx.notify();
    }

    /// The latest load failed, for `error`.
    pub(in crate::workspace) fn failed(&mut self, error: String, cx: &mut Context<Self>) {
        self.error = Some(error);
        cx.notify();
    }

    /// The commands agent `agent` just said it offers in `directory`, in
    /// place of those known, keeping its conversations.
    pub(in crate::workspace) fn learn_commands(
        &mut self,
        directory: PathBuf,
        agent: String,
        commands: Vec<AgentCommandInfo>,
        cx: &mut Context<Self>,
    ) {
        match self
            .catalog
            .as_mut()
            .filter(|catalog| catalog.directory == directory && catalog.agent == agent)
        {
            Some(catalog) => catalog.commands = commands,
            None => {
                self.catalog = Some(AgentCatalog {
                    agent,
                    directory,
                    conversations: Vec::new(),
                    commands,
                })
            }
        }
        cx.notify();
    }

    /// Set the catalog outright, as a finished load would.
    #[cfg(test)]
    pub(in crate::workspace) fn set(&mut self, catalog: AgentCatalog, cx: &mut Context<Self>) {
        self.generation += 1;
        self.loading = false;
        self.error = None;
        self.keep(catalog, cx);
    }
}
