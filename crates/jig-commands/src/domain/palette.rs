//! The ⌘K input's lists as data: the earlier conversations, the agent's
//! commands and the models, and which rows match what's typed.

/// An earlier conversation with the agent, as the palette lists it.
#[derive(Clone, Debug, PartialEq)]
pub struct PastConversation {
    pub id: String,
    pub title: String,
    /// When it last changed, e.g. "2 h ago".
    pub when: String,
}

/// An agent with one of its models, as the chip offers it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelChoice {
    /// The agent's key.
    pub agent: Option<String>,
    /// What it's listed under: the agent, e.g. "Claude Code".
    pub group: String,
    /// The model alone, e.g. "Opus 5.5".
    pub name: String,
    /// `provider/model` for OpenCode, the agent's own name for other
    /// agents; `None` for the agent's default.
    pub model: Option<String>,
    /// As listed, e.g. "Claude Code · Opus 5.5".
    pub label: String,
}

/// What the chip offers, and which is in use.
#[derive(Clone, Debug, Default)]
pub struct ModelMenu {
    pub choices: Vec<ModelChoice>,
    pub current: Option<usize>,
}

impl ModelMenu {
    pub fn current(&self) -> Option<&ModelChoice> {
        self.choices.get(self.current?)
    }

    /// The agents, in the order they're listed.
    pub fn groups(&self) -> Vec<&str> {
        let mut groups: Vec<&str> = Vec::new();
        for choice in &self.choices {
            if !groups.contains(&choice.group.as_str()) {
                groups.push(&choice.group);
            }
        }
        groups
    }
}

/// Picking a model: first the agent, then its models.
pub struct Picking {
    /// The list query to restore after.
    pub query: String,
    /// The agent chosen, once one is.
    pub group: Option<String>,
}

/// One of the agent's commands or skills, run with "/name".
#[derive(Clone, Debug, PartialEq)]
pub struct AgentCommandInfo {
    pub name: String,
    pub description: String,
    pub skill: bool,
}

/// A row of the list.
#[derive(Clone, Debug, PartialEq)]
pub enum Row {
    /// The typed text, as a message for the agent.
    Message(String),
    Conversation(usize),
    Command(usize),
    /// An agent to pick models from, by its place in the list.
    Group(usize),
    /// A model to pick, while picking one.
    Model(usize),
}

impl Row {
    pub fn section(&self) -> &'static str {
        match self {
            Row::Message(_) => "Message",
            Row::Conversation(_) => "Conversations",
            Row::Command(_) => "Commands",
            Row::Group(_) => "Agents",
            Row::Model(_) => "Models",
        }
    }
}

/// Picking a model: the agents whose name has the typed words, then the
/// models that do, so a model can be typed straight away; or, once an
/// agent is picked (`group`), its models.
pub fn model_rows(menu: &ModelMenu, group: Option<&str>, query: &str) -> Vec<Row> {
    let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    let has = |text: &str| {
        let text = text.to_lowercase();
        words.iter().all(|word| text.contains(word))
    };
    match group {
        Some(group) => menu
            .choices
            .iter()
            .enumerate()
            .filter(|(_, choice)| choice.group == group && has(&choice.name))
            .map(|(index, _)| Row::Model(index))
            .collect(),
        None => {
            let groups: Vec<Row> = menu
                .groups()
                .iter()
                .enumerate()
                .filter(|(_, group)| has(group))
                .map(|(index, _)| Row::Group(index))
                .collect();
            let models = menu
                .choices
                .iter()
                .enumerate()
                .filter(|(_, choice)| !words.is_empty() && has(&choice.label))
                .map(|(index, _)| Row::Model(index));
            groups.into_iter().chain(models).collect()
        }
    }
}

/// The typed message, then the earlier conversations it matches (all of
/// them before anything is typed).
pub fn conversation_rows(conversations: &[PastConversation], query: &str) -> Vec<Row> {
    let query = query.trim();
    let lower = query.to_lowercase();
    let mut rows = Vec::new();
    if !query.is_empty() {
        rows.push(Row::Message(query.to_string()));
    }
    rows.extend(
        conversations
            .iter()
            .enumerate()
            .filter(|(_, conversation)| {
                lower
                    .split_whitespace()
                    .all(|word| conversation.title.to_lowercase().contains(word))
            })
            .map(|(index, _)| Row::Conversation(index)),
    );
    rows
}

/// After "/": the commands and skills whose names match. Text after the
/// name is the command's arguments. With nothing matching, the text goes to
/// the agent as it is.
pub fn command_rows(commands: &[AgentCommandInfo], query: &str) -> Vec<Row> {
    let typed = query.trim_start().trim_start_matches('/');
    let (name, arguments) = typed.split_once(' ').unwrap_or((typed, ""));
    let name = name.to_lowercase();
    let mut matches: Vec<(u32, usize)> = commands
        .iter()
        .enumerate()
        .filter_map(|(index, command)| {
            let own = command.name.to_lowercase();
            if arguments.is_empty() {
                score(&own, &name).map(|score| (score, index))
            } else {
                (own == name).then_some((0, index))
            }
        })
        .collect();
    matches.sort();
    let mut rows: Vec<Row> = matches
        .into_iter()
        .map(|(_, index)| Row::Command(index))
        .collect();
    if rows.is_empty() {
        rows.push(Row::Message(query.trim().to_string()));
    }
    rows
}

/// What follows a command's name in `query`: its arguments.
pub fn command_arguments(query: &str) -> String {
    query
        .trim_start()
        .split_once(' ')
        .map(|(_, arguments)| arguments.trim().to_string())
        .unwrap_or_default()
}

/// The rest of `name` after `typed`, when `typed` is how it starts, in any
/// case; `None` when it isn't, or there's nothing left.
pub fn completion<'a>(name: &'a str, typed: &str) -> Option<&'a str> {
    let mut rest = name.char_indices();
    for wanted in typed.chars() {
        let (_, got) = rest.next()?;
        if !got.to_lowercase().eq(wanted.to_lowercase()) {
            return None;
        }
    }
    let (at, _) = rest.next()?;
    (!typed.is_empty()).then(|| &name[at..])
}

/// Lower is better: prefix, then substring, then a subsequence ranked by how
/// spread out its characters are. `None` when `query` isn't a subsequence.
fn score(name: &str, query: &str) -> Option<u32> {
    if name.starts_with(query) {
        return Some(0);
    }
    if name.contains(query) {
        return Some(1);
    }
    let mut gaps = 0;
    let mut chars = name.chars();
    for wanted in query.chars() {
        let mut skipped = 0;
        loop {
            match chars.next() {
                Some(c) if c == wanted => break,
                Some(_) => skipped += 1,
                None => return None,
            }
        }
        gaps += skipped;
    }
    Some(2 + gaps)
}
