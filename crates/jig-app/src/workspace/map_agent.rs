//! The agent on the Map. ⌘K there opens the input beside the boxes
//! selected, and the conversation it starts is told about them and the
//! level showing (`MapView::question`), never sent code. The agent
//! answers by drawing, with the Map's tools (`map::lens`), and a short
//! text whose links fly the Map to what they name. Each drawing is a row
//! of the conversation, shown on the Map or not, pinned or toured from
//! there; it belongs to the conversation, going when it closes and coming
//! back when it's picked up again. Explain this level is a Map action:
//! Jig's own instruction, in a normal conversation.

use gpui_kit::*;
use jig_ai::agent::Turn;
use jig_commands::{Bubble, ChatEntry, DrawingAction};
use serde_json::Value;

use super::Workspace;
use super::agent::TurnInput;
use super::commands::CommandRun;
use crate::map::lens::{self, Call, Drawing, Kind};
use crate::map::view::lens::Source;
use crate::map::view::{MapView, ProposalChoice};

/// A drawing of the conversation's, and whether it shows on the Map.
#[derive(Clone, Debug)]
pub(super) struct Drawn {
    pub(super) drawing: Drawing,
    pub(super) shown: bool,
    /// The pin it was saved as.
    pub(super) pin: Option<String>,
}

impl Workspace {
    /// A conversation started on the Map: the agent asked `instruction`
    /// about what's selected there and the level showing, beside the
    /// selection.
    pub(super) fn start_map_conversation(
        &mut self,
        name: Option<String>,
        instruction: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(view) = self.map_showing().cloned() else {
            return;
        };
        if self.agent_edit_here() {
            return self.show_note(
                "Accept or reject the agent's edit first.".into(),
                window,
                cx,
            );
        }
        let question = view.read(cx).question();
        let prompt = map_prompt(&instruction, &question.context);
        self.start_map_run(name, instruction, prompt, question.beside, window, cx);
    }

    /// A conversation from the Map named `name`, asking `prompt` (shown in
    /// the transcript as `instruction`), opened beside `beside`.
    pub(super) fn start_map_run(
        &mut self,
        name: Option<String>,
        instruction: String,
        prompt: String,
        beside: Option<Point<Pixels>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(view) = self.map_showing().cloned() else {
            return;
        };
        if self.agent_edit_here() {
            return self.show_note(
                "Accept or reject the agent's edit first.".into(),
                window,
                cx,
            );
        }
        let directory = view.read(cx).root().to_path_buf();
        self.close_chat(window, cx);
        self.next_run_id += 1;
        let id = self.next_run_id;
        let asked = name
            .clone()
            .unwrap_or_else(|| instruction.trim().to_string());
        let agent = self.new_agent_run(name, asked, directory, window, cx);
        let anchor = self.map_chat_anchor(beside, window);
        self.chat = Some(CommandRun {
            id,
            bubble: Bubble::Message(String::new()),
            anchor,
            preview: None,
            agent: Some(agent),
            _task: None,
        });
        self.start_turn(id, TurnInput::Prompt(prompt), window, cx);
    }

    /// Make Task on a sketch: the agent asked, with the sketch as context,
    /// to write a task for the user to agree to before it's saved.
    pub(super) fn start_sketch_conversation(
        &mut self,
        name: String,
        instruction: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(view) = self.map_showing().cloned() else {
            return;
        };
        let question = view.read(cx).question();
        let prompt = format!(
            "{}\n\n{}\n\nRead the code before you answer, so every place you link is real.",
            instruction.trim(),
            question.context
        );
        self.start_map_run(Some(name), instruction, prompt, question.beside, window, cx);
    }

    /// Implement on a sketch: its task run by the agent, every edit
    /// reviewed, the Map watching it turn solid.
    pub(super) fn implement_sketch_task(
        &mut self,
        task: &str,
        sketch: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(view) = self.map_showing().cloned() else {
            return;
        };
        let root = view.read(cx).root().to_path_buf();
        let task = match jig_ai::tasks::load(&root, task) {
            Ok(task) => task,
            Err(_) => {
                return self.show_note(
                    "The sketch's task isn't in the planner any more.".into(),
                    window,
                    cx,
                );
            }
        };
        let ask = super::planner::implement_ask(&task);
        let prompt = format!(
            "{}\n\n{}\n\nThe task was made from my sketch on Jig's Map ({}): build what it shows, naming new code as its boxes are labelled, and check each todo off with update_task as it's done.",
            ask.instruction,
            task.context(),
            crate::map::sketch::link(sketch)
        );
        self.start_map_run(ask.name, ask.instruction, prompt, None, window, cx);
    }

    /// Show sketch `name` on the Map, from a task's link to it.
    pub(super) fn show_sketch(&mut self, name: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.remember_location(cx);
        self.leave_tab(cx);
        let Some(view) = self.open_map_pane(window, cx) else {
            return;
        };
        let name = name.to_string();
        self.read_and(&view, cx, move |view, cx| view.go_to_sketch(&name, cx));
    }

    /// Where a conversation on the Map opens: just right of the selection
    /// when the window has room there, else at its right edge.
    pub(super) fn map_chat_anchor(
        &self,
        beside: Option<Point<Pixels>>,
        window: &Window,
    ) -> Point<Pixels> {
        let width = px(jig_commands::chat::WIDTH);
        let edge = window.viewport_size().width - self.docked_chat_width();
        beside
            .map(|at| at + point(px(28.), px(-4.)))
            .filter(|at| at.x + width + px(8.) <= edge)
            .unwrap_or_else(|| {
                point(
                    (edge - width - px(24.)).max(px(8.)),
                    px(super::TITLE_BAR_HEIGHT + 48.),
                )
            })
    }

    /// The agent drew on the Map (`tool`, given `input`): a row in the
    /// conversation and the drawing on the Map, once its places are known
    /// there; `map_clear` hides what it drew.
    pub(super) fn agent_drew(
        &mut self,
        id: u64,
        tool: &str,
        input: &Value,
        cx: &mut Context<Self>,
    ) {
        let call = match lens::call(tool, input) {
            Ok(call) => call,
            Err(_) => return,
        };
        // The tool turned down names it couldn't find; so does the Map.
        if let Call::Draw(drawing) = &call
            && let Some(pane) = self.map.as_ref()
            && !pane.view.read(cx).draws(drawing)
        {
            return;
        }
        let Some(agent) = self.agent_run(id) else {
            return;
        };
        match call {
            Call::Clear => {
                for drawn in &mut agent.drawings {
                    drawn.shown = false;
                }
                for entry in &mut agent.entries {
                    if let ChatEntry::Drawing { shown, .. } = entry {
                        *shown = false;
                    }
                }
                agent.entries.push(ChatEntry::Did("Cleared the Map".into()));
            }
            Call::Draw(drawing) => {
                let at = agent.drawings.len();
                agent.entries.push(drawing_entry(at, &drawing, true, false));
                agent.drawings.push(Drawn {
                    drawing,
                    shown: true,
                    pin: None,
                });
            }
        }
        agent.scroll.scroll_to_bottom();
        self.sync_map_drawings(cx);
        cx.notify();
    }

    /// Hand the Map the conversation's drawings that show.
    pub(super) fn sync_map_drawings(&self, cx: &mut App) {
        let Some(pane) = self.map.as_ref() else {
            return;
        };
        let drawings: Vec<(usize, Drawing)> = self
            .chat
            .as_ref()
            .and_then(|run| run.agent.as_ref())
            .map(|agent| {
                agent
                    .drawings
                    .iter()
                    .enumerate()
                    .filter(|(_, drawn)| drawn.shown)
                    .map(|(at, drawn)| (at, drawn.drawing.clone()))
                    .collect()
            })
            .unwrap_or_default();
        let bounds = self
            .chat
            .as_ref()
            .and_then(|run| run.agent.as_ref())
            .map(|agent| agent.frame_bounds());
        pane.view.update(cx, |view, cx| {
            view.set_conversation_bounds(bounds);
            view.set_drawings(drawings, cx)
        });
    }

    /// A drawing's row asked to show or hide it, pin it or tour it.
    pub(super) fn act_on_drawing(
        &mut self,
        at: usize,
        action: DrawingAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(agent) = self.chat.as_mut().and_then(|run| run.agent.as_mut()) else {
            return;
        };
        let Some(drawn) = agent.drawings.get(at).cloned() else {
            return;
        };
        match action {
            DrawingAction::Toggle => self.set_drawing_shown(at, !drawn.shown, cx),
            DrawingAction::Pin => {
                let Some(pane) = self.map.as_ref() else {
                    return self.show_note("Show the Map first, to pin to it.".into(), window, cx);
                };
                let view = pane.view.clone();
                let pinned = match &drawn.pin {
                    Some(name) => {
                        view.update(cx, |view, cx| view.unpin(name, cx));
                        Ok(None)
                    }
                    None => view
                        .update(cx, |view, cx| view.pin(&drawn.drawing, cx))
                        .map(Some),
                };
                let Some(agent) = self.chat.as_mut().and_then(|run| run.agent.as_mut()) else {
                    return;
                };
                match pinned {
                    Ok(pin) => {
                        let on = pin.is_some();
                        agent.drawings[at].pin = pin;
                        if let Some(ChatEntry::Drawing { pinned, .. }) =
                            drawing_row(&mut agent.entries, at)
                        {
                            *pinned = on;
                        }
                    }
                    Err(error) => agent.entries.push(ChatEntry::Error(format!("{error:#}"))),
                }
            }
            DrawingAction::Tour => {
                self.set_drawing_shown(at, true, cx);
                let start = move |view: &mut MapView, cx: &mut Context<MapView>| {
                    view.start_tour(Source::Conversation(at), cx)
                };
                match self.map_showing().cloned() {
                    Some(view) => view.update(cx, start),
                    None => {
                        self.leave_tab(cx);
                        if let Some(view) = self.open_map_pane(window, cx) {
                            self.sync_map_drawings(cx);
                            self.read_and(&view, cx, start);
                        }
                    }
                }
            }
        }
        cx.notify();
    }

    fn set_drawing_shown(&mut self, at: usize, on: bool, cx: &mut App) {
        let Some(agent) = self.chat.as_mut().and_then(|run| run.agent.as_mut()) else {
            return;
        };
        if let Some(drawn) = agent.drawings.get_mut(at) {
            drawn.shown = on;
        }
        if let Some(ChatEntry::Drawing { shown, .. }) = drawing_row(&mut agent.entries, at) {
            *shown = on;
        }
        self.sync_map_drawings(cx);
    }

    /// The Map's descriptions kept or rejected: they leave the Map, the
    /// row saying which.
    pub(super) fn drawing_settled(&mut self, at: usize, kept: bool, cx: &mut Context<Self>) {
        self.set_drawing_settled(at, if kept { "Kept" } else { "Rejected" }, cx);
    }

    fn set_drawing_settled(&mut self, at: usize, word: &str, cx: &mut Context<Self>) {
        self.set_drawing_shown(at, false, cx);
        if let Some(agent) = self.chat.as_mut().and_then(|run| run.agent.as_mut())
            && let Some(ChatEntry::Drawing { settled, .. }) = drawing_row(&mut agent.entries, at)
        {
            *settled = Some(word.to_string());
        }
        cx.notify();
    }

    /// What to do with the agent's proposal: a task of it, or it made,
    /// asked in the conversation; or it's dismissed.
    pub(super) fn proposal_chosen(
        &mut self,
        at: usize,
        choice: ProposalChoice,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(agent) = self.chat.as_ref().and_then(|run| run.agent.as_ref()) else {
            return;
        };
        let Some(drawn) = agent.drawings.get(at) else {
            return;
        };
        let title = match drawn.drawing.title.trim() {
            "" => "your proposal".to_string(),
            title => format!("the proposal “{title}”"),
        };
        let ask = match choice {
            ProposalChoice::Dismiss => {
                return self.set_drawing_settled(at, "Dismissed", cx);
            }
            // The Map made it the user's sketch.
            ProposalChoice::Adopt => {
                return self.set_drawing_settled(at, "Adopted", cx);
            }
            ProposalChoice::MakeTask => format!(
                "Save {title} as a task in Jig's task planner with create_task, laid out as the tool describes, its todos linking the code they touch."
            ),
            ProposalChoice::Implement => format!(
                "Implement {title}. If you saved it as a task, work through its todos and check each off."
            ),
        };
        if !agent.finished {
            return self.show_note("Wait for the agent to finish its turn.".into(), window, cx);
        }
        self.send_to_agent(ask, window, cx);
    }

    /// A link in the agent's words: the web in the browser; a place in the
    /// project flies the Map there while it shows, else opens its code.
    pub(super) fn open_agent_link(
        &mut self,
        link: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if link.contains("://") || link.starts_with("mailto:") {
            cx.open_url(link);
            return;
        }
        let link = link.strip_prefix("map:").unwrap_or(link);
        if let Some(view) = self.map_showing().cloned()
            && view.update(cx, |view, cx| view.fly_to(link, cx))
        {
            return;
        }
        let Some(project) = self
            .chat
            .as_ref()
            .and_then(|run| run.agent.as_ref())
            .map(|agent| agent.directory().to_path_buf())
        else {
            return;
        };
        if let Some(crate::planner::links::Target::Code(code)) =
            crate::planner::links::target(link, "")
        {
            let path = project.join(&code.path);
            self.open_code_link(&path, &code, window, cx);
        }
    }
}

/// The first message of a conversation on the Map: the question, then
/// what the Map shows, and how to answer.
fn map_prompt(instruction: &str, context: &str) -> String {
    format!(
        "{}\n\n{context}\n\nAnswer on the Map with the jig-map tools (read the code first, so every place you name is real), then reply in a few sentences, linking each place you mention as [name](path#Type.method).",
        instruction.trim()
    )
}

/// A drawing's row in the conversation.
pub(super) fn drawing_entry(at: usize, drawing: &Drawing, shown: bool, pinned: bool) -> ChatEntry {
    ChatEntry::Drawing {
        id: at,
        summary: drawing.summary(),
        shown,
        pinned,
        stepped: drawing.kind.stepped(),
        settled: None,
    }
}

fn drawing_row(entries: &mut [ChatEntry], at: usize) -> Option<&mut ChatEntry> {
    entries
        .iter_mut()
        .find(|entry| matches!(entry, ChatEntry::Drawing { id, .. } if *id == at))
}

/// An earlier conversation's lines as its transcript and drawings: what
/// it drew comes back, as it was left after any `map_clear`.
pub(super) fn transcript(turns: Vec<Turn>) -> (Vec<ChatEntry>, Vec<Drawn>) {
    let mut entries = Vec::new();
    let mut drawings: Vec<Drawn> = Vec::new();
    for turn in turns {
        match turn {
            Turn::Drew { tool, input } => match lens::call(&tool, &input) {
                Ok(Call::Clear) => {
                    for drawn in &mut drawings {
                        drawn.shown = false;
                    }
                    for entry in &mut entries {
                        if let ChatEntry::Drawing { shown, .. } = entry {
                            *shown = false;
                        }
                    }
                }
                Ok(Call::Draw(drawing)) => {
                    // Descriptions and proposals were settled then.
                    let shown = !matches!(drawing.kind, Kind::Describe | Kind::Propose);
                    entries.push(drawing_entry(drawings.len(), &drawing, shown, false));
                    drawings.push(Drawn {
                        drawing,
                        shown,
                        pin: None,
                    });
                }
                Err(_) => {}
            },
            turn => entries.push(super::agent::chat_entry(turn)),
        }
    }
    (entries, drawings)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use jig_ai::agent::Turn;
    use jig_commands::ChatEntry;

    use super::transcript;

    #[test]
    fn an_earlier_conversations_drawings_come_back() {
        let (entries, drawings) = transcript(vec![
            Turn::User("How does it work?".into()),
            Turn::Drew {
                tool: "map_trace".into(),
                input: json!({"title": "Flow", "steps": [{"place": "a.rs", "label": "Start"}]}),
            },
            Turn::Drew {
                tool: "map_clear".into(),
                input: json!({}),
            },
            Turn::Drew {
                tool: "map_highlight".into(),
                input: json!({"title": "These", "places": ["b.rs"]}),
            },
            Turn::Agent("Like [this](a.rs).".into()),
        ]);
        assert_eq!(drawings.len(), 2);
        assert!(!drawings[0].shown, "cleared");
        assert!(drawings[1].shown);
        assert!(matches!(
            &entries[1],
            ChatEntry::Drawing { shown: false, summary, .. } if summary == "Trace: Flow · 1 step"
        ));
        assert_eq!(entries.len(), 4);
    }
}
