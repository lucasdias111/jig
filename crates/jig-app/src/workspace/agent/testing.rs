//! Ways in for the UI tests: a conversation, a request or a sign-in without a real agent.

use super::turns::referred_to;
use super::*;

impl Workspace {
    /// An agent conversation between turns, without OpenCode behind it.
    #[cfg(test)]
    pub(in crate::workspace) fn open_test_conversation(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let id = self.start_agent_run(window, cx).expect("a project");
        let directory = self.agent_project(cx).expect("a project");
        let agent = self.new_agent_run(None, "Hello".into(), directory, window, cx);
        if let Some(run) = self.chat.as_mut().filter(|run| run.id == id) {
            run.agent = Some(agent);
        }
        let anchor = self.agent_anchor(window);
        if let Some(run) = self.chat.as_mut() {
            run.anchor = anchor;
        }
        cx.notify();
    }

    /// Hand the open test conversation a request, as if the agent had sent
    /// it mid-turn.
    #[cfg(test)]
    pub(in crate::workspace) fn test_agent_request(
        &mut self,
        event: AgentEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let id = self.chat.as_ref().map_or(0, |run| run.id);
        if let Some(agent) = self.agent_run(id) {
            agent.finished = false;
        }
        self.on_agent_message(id, Message::Event(event), window, cx);
    }

    /// As if the agent had said the user must sign in first.
    #[cfg(test)]
    pub(in crate::workspace) fn test_sign_in(
        &mut self,
        needed: SignInNeeded,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let id = self.chat.as_ref().map_or(0, |run| run.id);
        self.on_agent_message(id, Message::SignIn(needed), window, cx);
    }

    #[cfg(test)]
    pub(in crate::workspace) fn test_recent_conversations(
        &mut self,
        conversations: Vec<jig_ai::agent::Conversation>,
        cx: &mut Context<Self>,
    ) {
        let catalog = AgentCatalog {
            agent: self.picked_agent(cx).key.clone(),
            directory: self.agent_project(cx).unwrap(),
            conversations,
            commands: Vec::new(),
        };
        self.catalog
            .update(cx, |entity, cx| entity.set(catalog, cx));
        cx.notify();
    }

    /// How far the transcript is scrolled down, and how far it could be.
    #[cfg(test)]
    pub(in crate::workspace) fn agent_scroll(&self) -> (Pixels, Pixels) {
        let agent = self
            .chat
            .as_ref()
            .and_then(|run| run.agent.as_ref())
            .unwrap();
        (-agent.scroll.offset().y, agent.scroll.max_offset().y)
    }

    /// What the conversation last sent the agent, when it was a message.
    #[cfg(test)]
    pub(in crate::workspace) fn agent_prompt(&self) -> Option<String> {
        match self.chat.as_ref()?.agent.as_ref()?.last_input.as_ref()? {
            TurnInput::Prompt(prompt) => Some(prompt.clone()),
            TurnInput::Command { .. } => None,
        }
    }

    /// Whether the conversation waits on a go-ahead for something that only
    /// reads: for live tests, which allow those and nothing else.
    #[cfg(test)]
    pub(in crate::workspace) fn waiting_on_a_read(&self) -> bool {
        let Some(agent) = self.chat.as_ref().and_then(|run| run.agent.as_ref()) else {
            return false;
        };
        matches!(
            agent.pending.as_ref(),
            Some(Pending::Permission(request)) if only_reads(request, &agent.directory)
        )
    }

    /// What the conversation waits on the user for, in a word.
    #[cfg(test)]
    pub(in crate::workspace) fn agent_waiting_on(&self) -> Option<&'static str> {
        let agent = self.chat.as_ref()?.agent.as_ref()?;
        if agent.sign_in.is_some() {
            return Some("sign-in");
        }
        Some(match agent.pending.as_ref()? {
            Pending::Edit(_) => "edit",
            Pending::Permission(_) => "permission",
            Pending::Question { .. } => "question",
        })
    }

    /// The reply box's text, and the code its tokens name.
    #[cfg(test)]
    pub(in crate::workspace) fn agent_reply(&self, cx: &App) -> (String, Vec<CodeReference>) {
        let Some(agent) = self.chat.as_ref().and_then(|run| run.agent.as_ref()) else {
            return Default::default();
        };
        let content = agent.input.read(cx).content();
        (
            content.text().to_string(),
            referred_to(&content, &agent.references),
        )
    }

    #[cfg(test)]
    pub(in crate::workspace) fn agent_entries(&self) -> Vec<ChatEntry> {
        self.chat
            .as_ref()
            .and_then(|run| run.agent.as_ref())
            .map(|agent| agent.entries.clone())
            .unwrap_or_default()
    }
}
