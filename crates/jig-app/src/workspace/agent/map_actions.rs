//! The Map's actions (Index Domains and the like), each run as a conversation.

use super::*;

impl Workspace {
    /// Start the conversation for a Map action in project `directory`:
    /// named after the action, the agent asked `instruction`, beside the
    /// Map rather than at the code.
    pub(in crate::workspace) fn start_map_action(
        &mut self,
        action: Action,
        instruction: String,
        directory: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.agent_edit_here() {
            self.end_map_action(cx);
            return self.show_note(
                "Accept or reject the agent's edit first.".into(),
                window,
                cx,
            );
        }
        self.close_chat(window, cx);
        self.next_run_id += 1;
        let id = self.next_run_id;
        let title = action.title();
        let width = px(jig_commands::chat::WIDTH);
        let anchor = point(
            (window.viewport_size().width - width - px(24.)).max(px(8.)),
            px(super::super::TITLE_BAR_HEIGHT + 48.),
        );
        let mut agent = self.new_agent_run(Some(title.clone()), title, directory, window, cx);
        agent.map_action = Some(action);
        self.chat = Some(CommandRun {
            id,
            bubble: Bubble::Message(String::new()),
            anchor,
            preview: None,
            agent: Some(agent),
            _task: None,
        });
        self.start_turn(id, TurnInput::Prompt(instruction), window, cx);
    }

    /// The Map's action is over without an answer to read.
    pub(super) fn end_map_action(&self, cx: &mut App) {
        if let Some(pane) = self.map.as_ref() {
            pane.view.update(cx, |view, cx| view.action_ended(cx));
        }
    }

    /// The agent answered a Map action, with `map_propose_domains` (its
    /// `call`) or in its words `text`: the Map reads the answer as a
    /// proposal and shows it at Domain, or the conversation says why it
    /// couldn't.
    pub(super) fn answer_map_action(
        &mut self,
        action: Action,
        text: &str,
        call: Option<&Value>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(view) = self.map.as_ref().map(|pane| pane.view.clone()) else {
            return;
        };
        let read = view.update(cx, |view, cx| view.answer(action, text, call, cx));
        let proposed = read.is_ok();
        let entry = match read {
            Ok(()) => ChatEntry::Note(
                "It's on the Map as a proposal: keep it there (⌘↩), edit it, or reply here to redo parts."
                    .into(),
            ),
            Err(error) => ChatEntry::Error(format!(
                "The Map couldn't read domains from that: {error} Reply to ask again."
            )),
        };
        if let Some(agent) = self.chat.as_mut().and_then(|run| run.agent.as_mut()) {
            agent.entries.push(entry);
            agent.scroll.scroll_to_bottom();
        }
        if proposed {
            self.map_at(jig_map::Level::Domain, window, cx);
        }
        cx.notify();
    }
}
