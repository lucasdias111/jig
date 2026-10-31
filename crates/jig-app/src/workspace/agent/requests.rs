//! Answering what the agent asks: a question, a task to save, a go-ahead for a tool.

use super::*;

impl Workspace {
    /// Answer the question on screen with option `choice`, or with what's
    /// in the reply box: an option's number or the user's own words.
    /// Returns false when no question is waiting.
    pub(in crate::workspace) fn answer_agent_question(
        &mut self,
        choice: Option<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(agent) = self.chat.as_mut().and_then(|run| run.agent.as_mut()) else {
            return false;
        };
        let Some(Pending::Question { request, answers }) = agent.pending.as_mut() else {
            return false;
        };
        let question = &request.questions[answers.len()];
        let typed = agent.input.read(cx).value().trim().to_string();
        let answer = match choice {
            Some(index) => question.options.get(index).cloned(),
            None => typed
                .parse::<usize>()
                .ok()
                .and_then(|number| question.options.get(number.wrapping_sub(1)).cloned())
                .or_else(|| (!typed.is_empty()).then_some(typed)),
        };
        let Some(answer) = answer else {
            return true;
        };
        answers.push(answer);
        agent
            .input
            .update(cx, |input, cx| input.set_value("", window, cx));
        if answers.len() < request.questions.len() {
            cx.notify();
            return true;
        }
        let Some(Pending::Question { request, answers }) = agent.pending.take() else {
            return true;
        };
        agent.entries.push(ChatEntry::User(answers.join(" · ")));
        agent.scroll.scroll_to_bottom();
        if let Some(session) = agent.session.clone() {
            cx.background_executor()
                .spawn(async move {
                    let _ = session.answer(&request.id, Some(&answers));
                })
                .detach();
        }
        let id = self.chat.as_ref().map_or(0, |run| run.id);
        self.focus_agent_or_editor(window, cx);
        self.show_next_request(id, window, cx);
        cx.notify();
        true
    }

    /// Enter in the reply box while the agent asks to save a plan as a
    /// task: saves it when nothing's typed, otherwise turns it down with
    /// what's typed for the agent to follow. Returns false when it isn't
    /// asking that.
    pub(in crate::workspace) fn answer_task_question(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(run) = self.chat.as_mut() else {
            return false;
        };
        let id = run.id;
        let Some(agent) = run.agent.as_mut() else {
            return false;
        };
        if !matches!(&agent.pending, Some(Pending::Permission(request)) if request.task.is_some()) {
            return false;
        }
        let said = agent.input.read(cx).value().trim().to_string();
        if said.is_empty() {
            return self.settle_agent_request(true, window, cx);
        }
        let Some(Pending::Permission(request)) = agent.pending.take() else {
            return false;
        };
        agent
            .input
            .update(cx, |input, cx| input.set_value("", window, cx));
        agent.entries.push(ChatEntry::User(said.clone()));
        agent.scroll.scroll_to_bottom();
        if let Some(session) = agent.session.clone() {
            let note = session.hears_notes().then(|| said.clone());
            if note.is_none() {
                agent.follow_up = Some(said);
            }
            cx.background_executor()
                .spawn(async move {
                    let _ = session.reply(&request.id, false, note.as_deref());
                })
                .detach();
        }
        self.focus_agent_or_editor(window, cx);
        self.show_next_request(id, window, cx);
        cx.notify();
        true
    }

    /// Allow or turn down what the agent asked to do (a command, the web, a
    /// subagent), or decline its question. Returns false when it isn't
    /// waiting on one.
    pub(in crate::workspace) fn settle_agent_request(
        &mut self,
        allow: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(run) = self.chat.as_mut() else {
            return false;
        };
        let id = run.id;
        let Some(agent) = run.agent.as_mut() else {
            return false;
        };
        // Enter with words in the reply box answers the plan with them.
        let typed = !agent.input.read(cx).value().trim().is_empty();
        if allow
            && typed
            && matches!(&agent.pending, Some(Pending::Permission(request)) if request.task.is_some())
        {
            return self.answer_task_question(window, cx);
        }
        let session = agent.session.clone();
        match agent.pending.take() {
            Some(Pending::Permission(request)) => {
                if request.task.is_some() {
                    agent
                        .input
                        .update(cx, |input, cx| input.set_value("", window, cx));
                    if !allow {
                        agent
                            .entries
                            .push(ChatEntry::Note("Not saved as a task.".into()));
                    }
                    self.focus_agent_or_editor(window, cx);
                } else if !allow {
                    agent
                        .entries
                        .push(ChatEntry::Note(format!("Not allowed: {}", request.detail)));
                }
                if let Some(session) = session {
                    cx.background_executor()
                        .spawn(async move {
                            let _ = session.reply(&request.id, allow, None);
                        })
                        .detach();
                }
            }
            // A question is answered in the reply box; Esc declines it.
            Some(Pending::Question { request, .. }) if !allow => {
                agent
                    .entries
                    .push(ChatEntry::Note("Declined to answer.".into()));
                agent
                    .input
                    .update(cx, |input, cx| input.set_value("", window, cx));
                if let Some(session) = session {
                    cx.background_executor()
                        .spawn(async move {
                            let _ = session.answer(&request.id, None);
                        })
                        .detach();
                }
                self.focus_agent_or_editor(window, cx);
            }
            other => {
                agent.pending = other;
                return false;
            }
        }
        if let Some(agent) = self.agent_run(id) {
            agent.scroll.scroll_to_bottom();
        }
        self.show_next_request(id, window, cx);
        cx.notify();
        true
    }

    /// ⌘↩ or Always allow: let the request through, and others like it
    /// from now on, as the agent remembers them or else for the rest of the
    /// conversation. Returns false when no request is waiting.
    pub(in crate::workspace) fn allow_agent_request_always(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(run) = self.chat.as_mut() else {
            return false;
        };
        let id = run.id;
        let Some(agent) = run.agent.as_mut() else {
            return false;
        };
        let request = match agent.pending.take() {
            // Saving a plan has no "always": the next one is asked again.
            Some(Pending::Permission(request)) if request.task.is_some() => {
                agent.pending = Some(Pending::Permission(request));
                return self.settle_agent_request(true, window, cx);
            }
            Some(Pending::Permission(request)) => request,
            Some(edit @ Pending::Edit(_)) => {
                agent.pending = Some(edit);
                return self.accept_all_agent_edits(id, window, cx);
            }
            other => {
                agent.pending = other;
                return false;
            }
        };
        let agent_remembers = request.always.is_some();
        if !agent_remembers {
            agent.always_allowed.push(request.title.clone());
        }
        agent.entries.push(ChatEntry::Note(match &request.always {
            Some(always) => format!("{always}."),
            None => format!("{}: allowed for this conversation.", request.title),
        }));
        agent.scroll.scroll_to_bottom();
        if let Some(session) = agent.session.clone() {
            cx.background_executor()
                .spawn(async move {
                    let _ = if agent_remembers {
                        session.allow_always(&request.id)
                    } else {
                        session.reply(&request.id, true, None)
                    };
                })
                .detach();
        }
        self.show_next_request(id, window, cx);
        cx.notify();
        true
    }

    /// ⌘↩ on the agent's edit: accept it, and its later edits as they come.
    fn accept_all_agent_edits(
        &mut self,
        id: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.settle_agent_edit(true, Some(window), cx) {
            return false;
        }
        if let Some(agent) = self.agent_run(id) {
            agent.accept_edits = true;
            agent.entries.push(ChatEntry::Note(
                "Edits: accepted for the rest of this conversation.".into(),
            ));
            agent.scroll.scroll_to_bottom();
        }
        self.show_next_request(id, window, cx);
        cx.notify();
        true
    }

    /// One of the request's buttons: 0 allows, 1 allows always, 2 denies.
    /// For an edit: 0 accepts it, 1 accepts it and the rest, 2 rejects it.
    pub(super) fn decide_agent_request(
        &mut self,
        button: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let edit = self
            .chat
            .as_ref()
            .and_then(|run| run.agent.as_ref())
            .is_some_and(|agent| matches!(agent.pending, Some(Pending::Edit(_))));
        match (edit, button) {
            (true, 0) => self.settle_agent_edit(true, Some(window), cx),
            (true, 2) => self.settle_agent_edit(false, Some(window), cx),
            (false, 0) => self.settle_agent_request(true, window, cx),
            (_, 1) => self.allow_agent_request_always(window, cx),
            _ => self.settle_agent_request(false, window, cx),
        };
        self.focus_agent_or_editor(window, cx);
    }
}
