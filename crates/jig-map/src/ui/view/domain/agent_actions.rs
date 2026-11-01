//! The Map's actions on domains, run by the agent, and what it proposes.

use super::*;

impl MapView {
    /// The banner over Domain, if it has one: drafts to index, an action
    /// under way, or a proposal to review.
    pub(in crate::ui::view) fn domain_banner(&self) -> Option<String> {
        if self.level != Level::Domain {
            return None;
        }
        if let Some(action) = &self.domains.working {
            return Some(format!(
                "{} is under way in the agent's conversation…",
                action.title()
            ));
        }
        if let Some(proposal) = &self.domains.proposal {
            return Some(match &proposal.action {
                Action::Index => "The agent proposes these domains".into(),
                Action::Update(files) => format!(
                    "The agent proposes where {} go{}",
                    plural(files.len(), "new file"),
                    if files.len() == 1 { "es" } else { "" }
                ),
                Action::Split(name) => format!("The agent proposes how to split {name}"),
            });
        }
        if self.drafts() {
            return Some("Index Domains to name and describe these".into());
        }
        None
    }

    /// Start a Map action's conversation: what it asks, from the domains
    /// as they are.
    pub fn start_action(&mut self, action: Action, cx: &mut Context<Self>) {
        let instruction = match &action {
            Action::Index => {
                let Some(grouping) = self.grouping() else {
                    return;
                };
                actions::index_instruction(&crate::domain::domains::tools::grouping_hint(&grouping))
            }
            Action::Update(files) => {
                let Some(saved) = &self.domains.saved else {
                    return;
                };
                if files.is_empty() {
                    return self.say("Every file has a domain: there's nothing to place.", cx);
                }
                actions::update_instruction(files, saved)
            }
            Action::Split(name) => {
                let Some(saved) = &self.domains.saved else {
                    return;
                };
                actions::split_instruction(name, saved)
            }
        };
        self.domains.working = Some(action.clone());
        cx.emit(MapEvent::Ask(action, instruction));
        cx.notify();
    }

    /// Update Domains: the agent places what's unsorted.
    pub fn update_domains(&mut self, cx: &mut Context<Self>) {
        let files = self
            .domain_level()
            .map(|level| level.unsorted())
            .unwrap_or_default();
        self.start_action(Action::Update(files), cx);
    }

    /// The agent answered action `action` with `call`, its call of
    /// `map_propose_domains`, else in its words `reply`: its proposal
    /// shows, or why it can't.
    pub fn answer(
        &mut self,
        action: Action,
        reply: &str,
        call: Option<&serde_json::Value>,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        self.domains.working = None;
        let saved = self.domains.saved.as_ref();
        let proposal = match (call, self.project()) {
            (Some(call), Some(project)) => {
                let files = crate::domain::domains::tools::source_files(project);
                action.proposal_from_call(call, saved, &files)
            }
            _ => action
                .proposal(reply, saved)
                .map_err(|error| format!("{error:#}")),
        };
        if let Ok(domains) = &proposal {
            self.remember();
            self.domains.proposal = Some(Proposal {
                action,
                domains: domains.clone(),
            });
            self.domains_changed();
            if self.level == Level::Domain {
                let to = self.domain_fit();
                self.transition = Some(Transition::pan(self.camera(), to));
                self.set_camera(to);
            }
        }
        cx.notify();
        proposal.map(|_| ())
    }

    /// Keep the proposal: saved, and showing as the domains.
    pub fn keep_proposal(&mut self, cx: &mut Context<Self>) {
        let Some(proposal) = self.domains.proposal.clone() else {
            return;
        };
        self.remember();
        self.domains.proposal = None;
        self.save_domains(Some(proposal.domains), cx);
    }

    /// Discard the proposal; ⌘Z brings it back.
    pub fn discard_proposal(&mut self, cx: &mut Context<Self>) {
        if self.domains.proposal.is_none() {
            return;
        }
        self.remember();
        self.domains.proposal = None;
        self.domains_changed();
        cx.notify();
    }

    /// Splitting a domain asks the agent.
    pub fn split_domain(&mut self, domain: usize, cx: &mut Context<Self>) {
        let Some(name) = self
            .domains
            .saved
            .as_ref()
            .and_then(|saved| saved.domains.get(domain))
            .map(|domain| domain.name.clone())
        else {
            return;
        };
        self.start_action(Action::Split(name), cx);
    }

    /// Copy as Markdown: each domain's name, purpose and offers, then the
    /// flows.
    pub fn copy_markdown(&mut self, cx: &mut Context<Self>) {
        let Some(domains) = self.shown_domains() else {
            return;
        };
        let project = self
            .root
            .file_name()
            .map_or("Project".into(), |name| name.to_string_lossy().into_owned());
        cx.write_to_clipboard(ClipboardItem::new_string(domains.markdown(&project)));
        self.say("Copied the domains as Markdown.", cx);
    }
}
