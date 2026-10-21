//! The Map's actions on domains: what the agent is asked (Index Domains,
//! Update Domains, splitting a domain), each in a normal conversation the
//! user can watch and steer, and what its answer becomes: a proposal,
//! reviewed on the canvas before it's saved. Jig applies an update itself,
//! so the agent can place unsorted files and add domains but never move
//! what was confirmed.

use anyhow::{Result, bail};
use jig_ai::agent::{Asking, PermissionRequest};

use super::{Domain, Domains, Flow, Home};

/// Which action a conversation is, to read its answer back.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    Index,
    /// Placing these unsorted files.
    Update(Vec<String>),
    /// Splitting the domain of this name.
    Split(String),
}

impl Action {
    /// What the conversation is called.
    pub fn title(&self) -> String {
        match self {
            Action::Index => "Index Domains".into(),
            Action::Update(_) => "Update Domains".into(),
            Action::Split(name) => format!("Split {name}"),
        }
    }

    /// The domains its answer `reply` proposes, from `saved`: the TOML in
    /// it, for an agent that answered in words rather than with
    /// `map_propose_domains`.
    pub fn proposal(&self, reply: &str, saved: Option<&Domains>) -> Result<Domains> {
        let text = fenced_toml(reply);
        let answer = match self {
            Action::Update(_) => Answer::Update(toml::from_str(text)?),
            Action::Index | Action::Split(_) => Answer::Domains(toml::from_str(text)?),
        };
        self.apply(answer, saved)
    }

    /// The domains a call of `map_propose_domains` with `input` proposes,
    /// from `saved`, checked against `files`, every source file: what's
    /// wrong comes back for the agent to fix.
    pub fn proposal_from_call(
        &self,
        input: &serde_json::Value,
        saved: Option<&Domains>,
        files: &[String],
    ) -> Result<Domains, String> {
        let answer = super::tools::answer(self, input)?;
        super::tools::check(self, &answer, saved, files)?;
        self.apply(answer, saved)
            .map_err(|error| format!("{error:#}"))
    }

    /// `answer` made into the domains it proposes, from `saved`.
    fn apply(&self, answer: Answer, saved: Option<&Domains>) -> Result<Domains> {
        match (self, answer) {
            (Action::Index, Answer::Domains(mut domains)) => {
                if domains.domains.is_empty() {
                    bail!("The agent's answer has no domains in it.");
                }
                domains.tidy_flows();
                Ok(domains)
            }
            (Action::Update(unsorted), Answer::Update(update)) => {
                Ok(update.apply(saved.cloned().unwrap_or_default(), unsorted))
            }
            (Action::Split(name), Answer::Domains(split)) => {
                let Some(saved) = saved else {
                    bail!("There are no saved domains to split.");
                };
                let Some(ix) = saved.index_of(name) else {
                    bail!("{name} isn't a domain any more.");
                };
                if split.domains.len() < 2 {
                    bail!("The agent's answer doesn't split {name}.");
                }
                Ok(apply_split(saved.clone(), ix, split))
            }
            _ => bail!("The answer is for another action."),
        }
    }
}

/// What an answer says, read from TOML or from a tool call, before Jig
/// applies it.
#[derive(Debug)]
pub(super) enum Answer {
    /// Index Domains' or a split's: the domains, their flows and Shared.
    Domains(Domains),
    Update(Update),
}

/// Update Domains' answer: where each unsorted file goes, and any new
/// domain or flow it needs.
#[derive(Debug, Default, serde::Deserialize)]
pub(super) struct Update {
    #[serde(default)]
    pub(super) place: Vec<Place>,
    #[serde(default)]
    pub(super) new_domain: Vec<Domain>,
    #[serde(default)]
    pub(super) new_flow: Vec<Flow>,
}

#[derive(Debug, serde::Deserialize)]
pub(super) struct Place {
    pub(super) file: String,
    pub(super) domain: String,
}

impl Update {
    /// `saved` with the unsorted files placed: only those, only into a
    /// domain that's there or new, and new domains and flows added beside
    /// the old ones, never over them.
    fn apply(self, mut saved: Domains, unsorted: &[String]) -> Domains {
        let mut added = Vec::new();
        for domain in self.new_domain {
            if saved.index_of(&domain.name).is_none() && !is_shared(&domain.name) {
                added.push(domain.name.clone());
                saved.domains.push(Domain {
                    members: Vec::new(),
                    ..domain
                });
            }
        }
        for place in self.place {
            let file = place.file.trim().trim_start_matches("./").to_string();
            if !unsorted.contains(&file) {
                continue;
            }
            let home = if is_shared(&place.domain) {
                Home::Shared
            } else {
                match saved.index_of(place.domain.trim()) {
                    Some(ix) => Home::Domain(ix),
                    None => continue,
                }
            };
            saved.move_member(&file, home);
        }
        // A new domain nothing was placed in isn't kept.
        let empty: Vec<String> = saved
            .domains
            .iter()
            .filter(|domain| domain.members.is_empty() && added.contains(&domain.name))
            .map(|domain| domain.name.clone())
            .collect();
        saved.domains.retain(|domain| !empty.contains(&domain.name));
        for flow in self.new_flow {
            if empty.contains(&flow.from) || empty.contains(&flow.to) {
                continue;
            }
            if !saved
                .flows
                .iter()
                .any(|f| f.from == flow.from && f.to == flow.to)
            {
                saved.flows.push(flow);
            }
        }
        saved.tidy_flows();
        saved
    }
}

pub(super) fn is_shared(name: &str) -> bool {
    name.trim().eq_ignore_ascii_case("shared")
}

/// Domain `ix` of `saved` replaced by the domains `split` makes of it,
/// where it was; flows to and from it replaced by those `split` gives.
fn apply_split(mut saved: Domains, ix: usize, split: Domains) -> Domains {
    let old = saved.domains[ix].name.clone();
    let others: Vec<String> = saved
        .domains
        .iter()
        .filter(|domain| domain.name != old)
        .map(|domain| domain.name.clone())
        .collect();
    let new: Vec<Domain> = split
        .domains
        .into_iter()
        .filter(|domain| !others.contains(&domain.name))
        .collect();
    let names: Vec<String> = new.iter().map(|domain| domain.name.clone()).collect();
    saved.domains.splice(ix..=ix, new);
    saved
        .flows
        .retain(|flow| flow.from != old && flow.to != old);
    for flow in split.flows {
        if names.contains(&flow.from) || names.contains(&flow.to) {
            saved.flows.push(flow);
        }
    }
    saved.tidy_flows();
    saved
}

/// The TOML in an answer: its fenced block, else all of it.
pub fn fenced_toml(reply: &str) -> &str {
    for fence in ["```toml", "```"] {
        if let Some(start) = reply.find(fence) {
            let body = &reply[start + fence.len()..];
            let body = body.trim_start_matches(|c: char| c != '\n');
            if let Some(end) = body.find("```") {
                return body[..end].trim();
            }
        }
    }
    reply.trim()
}

/// The answer's format, as the agent is shown it.
const DOMAINS_FORMAT: &str = r#"```toml
[[domain]]
name = "…"
purpose = "…"
offers = ["…"]
why = "…"
members = ["path/to/file.ts", "path/to/File.java#Type", "path/to/folder/"]

[[flow]]
from = "<domain name>"
to = "<domain name>"
label = "…"

[shared]
members = ["…"]
```

A folder ending in `/` stands for every file under it."#;

/// How every action asks the agent to work: with tools that need no
/// go-ahead.
const TOOLS: &str = "Work only with these, so nothing waits for the user's go-ahead: the map_project tool (Jig's outline of the project: its grouping, every source file, and each file's purpose, types, functions, imports and calls), your own tools for reading files and searching them, and the map_propose_domains tool to answer. Don't run shell commands and don't start subagents. Only read; change nothing.";

/// What to do with the answer, and the fallback for an agent without the
/// tool.
fn answer_with(action: &str, format: &str) -> String {
    format!(
        "Answer by calling map_propose_domains with \"action\": {action}. It checks the answer against the project: if it reports problems, fix them and call it again with the whole answer. Once it's accepted, reply in a sentence or two; the user sees the proposal on the Map.

Only if you have no map_propose_domains tool, reply instead with only this TOML, in one fenced block:

{format}"
    )
}

/// What Index Domains asks, with Jig's grouping as a hint.
pub fn index_instruction(hint: &str) -> String {
    format!(
        "You are indexing this project's business domains for Jig's Map. Its top level shows the project as a few business areas, for programmers finding their way and for non-engineers (a product manager, a client) who never read code.

{TOOLS}

Below is Jig's own grouping of the project: source files, including ones Jig's index can't read, grouped by folder and calls, with the words their function names use most and the calls between groups. It's a starting hint, not an answer: folders often cut across business areas, and the words are guesses.

Do this:
1. Call map_project with a group or folder for each file's purpose, types, functions and calls, and read enough of the code to know what the project does for the people who use it: entry points (routes, controllers, screens, commands, jobs), the main entities, and names in the domain's own language (keep non-English business terms, such as Danish ones, and explain them).
2. Group the files into 3 to 9 business domains. Each file belongs to one domain; when a file mixes several, name a type instead (`path#Type`). Code used everywhere (configuration, logging, HTTP plumbing, shared utilities, build scripts) goes in shared, not in a domain. Every source file must end up in a domain or in shared; a folder ending in `/` stands for every file under it.
3. For each domain write a name a non-engineer understands (two or three words), a purpose of one or two plain sentences with no code terms, what it offers people in plain words (screens, commands, jobs, emails: what someone can do or gets, never routes, URLs, endpoints or code names), and one sentence on why these files belong together.
4. For the arrows between domains, say what flows along each in a few plain words (\"books rooms for\", \"sends invites to\").

{}

{hint}",
        answer_with("\"index\"", DOMAINS_FORMAT)
    )
}

/// What Update Domains asks: place only the unsorted files.
pub fn update_instruction(unsorted: &[String], saved: &Domains) -> String {
    let files: Vec<String> = unsorted.iter().map(|file| format!("- {file}")).collect();
    format!(
        "You are updating this project's business domains for Jig's Map. They were indexed earlier and confirmed by a person; the saved result is below. Since then these files appeared that no domain holds:

{}

{TOOLS}

Do this:
1. Read the unsorted files, and as much around them as you need (map_project with their folder gives each file's purpose, types, functions and calls), to know what they do for the people who use the project.
2. Put each in the existing domain it belongs to, or in Shared if it's code used everywhere. Only when none fits, propose one new domain (a name a non-engineer understands, a plain purpose, what it offers people in plain words with no routes or code names, why; keep non-English business terms and explain them). Never move or rename what is already placed, and don't change existing domains.
3. Say if the new code adds a flow between domains.

{}

The saved domains:

{}",
        files.join("\n"),
        answer_with(
            "\"update\", a placement for every unsorted file, and new_domains and new_flows only if needed",
            "```toml
[[place]]
file = \"…\"
domain = \"<existing or new domain name, or Shared>\"

[[new_domain]]   # only if needed
name = \"…\"
purpose = \"…\"
offers = [\"…\"]
why = \"…\"

[[new_flow]]     # only if needed
from = \"…\"
to = \"…\"
label = \"…\"
```"
        ),
        saved.to_toml()
    )
}

/// What splitting domain `name` asks.
pub fn split_instruction(name: &str, saved: &Domains) -> String {
    format!(
        "You are splitting one of this project's business domains for Jig's Map: \"{name}\" holds too much. The saved domains are below; the others were confirmed by a person and stay as they are.

{TOOLS}

Do this:
1. Read the files of \"{name}\", as much as you need (map_project with a folder gives each file's purpose, types, functions and calls), to see the business areas in it.
2. Split it into two or more domains, each with a name a non-engineer understands (two or three words), a purpose of one or two plain sentences with no code terms, what it offers people in plain words (no routes or code names), why its files belong together, and its members. Every member of \"{name}\" must end up in one of them. Don't reuse another domain's name; keep non-English business terms and explain them.
3. Give the flows between the new domains, and between them and the other domains, each in a few plain words.

{}

The saved domains:

{}",
        answer_with(
            &format!("\"split\", \"domain\": \"{name}\", and just the new domains and their flows"),
            DOMAINS_FORMAT
        ),
        saved.to_toml()
    )
}

/// Whether a shell command only reads: every command in it is one that
/// looks and changes nothing, with no writing to files. Map actions let
/// these run without asking.
pub fn read_only_command(command: &str) -> bool {
    const READERS: &[&str] = &[
        "ls", "cat", "head", "tail", "grep", "egrep", "fgrep", "rg", "ag", "find", "fd", "wc",
        "sort", "uniq", "tree", "file", "stat", "pwd", "echo", "cut", "tr", "nl", "less", "more",
        "du", "basename", "dirname", "realpath", "readlink", "which", "awk", "sed", "jq", "column",
        "diff", "cd", "true", "printf", "xargs",
    ];
    const GIT_READERS: &[&str] = &[
        "ls-files",
        "log",
        "show",
        "status",
        "diff",
        "grep",
        "blame",
        "branch",
        "rev-parse",
        "shortlog",
        "ls-tree",
        "cat-file",
    ];
    let Some(parts) = simple_commands(command) else {
        return false;
    };
    parts.iter().map(|part| part.as_str()).all(|part| {
        let words: Vec<&str> = part.split_whitespace().collect();
        let Some(&program) = words.first() else {
            return true;
        };
        let program = program.rsplit('/').next().unwrap_or(program);
        match program {
            "git" => words
                .iter()
                .skip(1)
                .find(|word| !word.starts_with('-'))
                .is_some_and(|sub| GIT_READERS.contains(sub)),
            "find" | "fd" => !words.iter().any(|word| {
                matches!(
                    *word,
                    "-exec" | "-execdir" | "-delete" | "-ok" | "-okdir" | "-x" | "-X"
                )
            }),
            "sed" => !words.iter().any(|word| word.starts_with("-i")),
            "awk" => !part.contains("system"),
            "xargs" => words
                .iter()
                .skip(1)
                .find(|word| !word.starts_with('-'))
                .is_some_and(|next| READERS.contains(next) && *next != "xargs"),
            _ => READERS.contains(&program),
        }
    })
}

/// A command's simple commands, split at `|`, `;`, `&` and new lines
/// outside quotes, with redirections that throw output away
/// (`2>/dev/null`, `>/dev/null`, `&>/dev/null`, `2>&1`) left out. `None`
/// when it writes anywhere else or runs a command inside it.
fn simple_commands(command: &str) -> Option<Vec<String>> {
    let chars: Vec<char> = command.chars().collect();
    let mut parts = vec![String::new()];
    let mut quote: Option<char> = None;
    let mut at = 0;
    while at < chars.len() {
        let c = chars[at];
        let next = chars.get(at + 1).copied();
        let part = parts.last_mut().expect("one part at least");
        match quote {
            Some(open) => {
                // Double quotes still substitute commands.
                if open == '"' && (c == '`' || (c == '$' && next == Some('('))) {
                    return None;
                }
                if c == open {
                    quote = None;
                } else if c == '\\'
                    && open == '"'
                    && let Some(next) = next
                {
                    part.push(c);
                    part.push(next);
                    at += 2;
                    continue;
                }
                part.push(c);
            }
            None => match c {
                '\'' | '"' => {
                    quote = Some(c);
                    part.push(c);
                }
                '`' => return None,
                '$' | '<' if next == Some('(') => return None,
                '\\' => {
                    part.push(c);
                    if let Some(next) = next {
                        part.push(next);
                        at += 1;
                    }
                }
                '&' if next == Some('>') => {}
                '|' | ';' | '&' | '\n' => parts.push(String::new()),
                '>' => {
                    // The descriptor written against it goes too.
                    while part.ends_with(|c: char| c.is_ascii_digit()) {
                        part.pop();
                    }
                    let rest: String = chars[at + 1..].iter().collect();
                    if rest.starts_with("&1") || rest.starts_with("&2") {
                        at += 3;
                        continue;
                    }
                    let target = rest.trim_start();
                    let spaces = rest.chars().count() - target.chars().count();
                    let quiet = target.strip_prefix("/dev/null").is_some_and(|after| {
                        after
                            .chars()
                            .next()
                            .is_none_or(|c| c.is_whitespace() || matches!(c, ';' | '|' | '&' | ')'))
                    });
                    if !quiet {
                        return None;
                    }
                    part.push(' ');
                    at += 1 + spaces + "/dev/null".len();
                    continue;
                }
                _ => part.push(c),
            },
        }
        at += 1;
    }
    Some(
        parts
            .into_iter()
            .map(|part| part.trim().to_string())
            .filter(|part| !part.is_empty())
            .collect(),
    )
}

/// Whether `request` may go ahead without asking during a Map action:
/// it only reads, whatever the agent titles it. A shell command that only
/// reads, reading and searching, or a subagent that only looks around.
pub fn only_reads(request: &PermissionRequest) -> bool {
    match request.kind {
        Asking::Command => read_only_command(&request.detail),
        Asking::Read => true,
        Asking::Subagent { explores } => explores,
        Asking::Web | Asking::Other => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn saved() -> Domains {
        toml::from_str(
            r#"
[[domain]]
name = "Bookings"
purpose = "Book places."
members = ["src/bookings/"]

[[domain]]
name = "Events"
purpose = "List events."
members = ["src/events/"]

[[flow]]
from = "Events"
to = "Bookings"
label = "supplies events to"

[shared]
members = ["src/util.ts"]
"#,
        )
        .unwrap()
    }

    #[test]
    fn the_toml_is_read_from_its_fenced_block() {
        let reply = "Here you go:\n\n```toml\n[[domain]]\nname = \"A\"\n```\nDone.";
        assert_eq!(fenced_toml(reply), "[[domain]]\nname = \"A\"");
        assert_eq!(fenced_toml("[[domain]]"), "[[domain]]");
    }

    #[test]
    fn index_proposes_what_the_agent_answered() {
        let reply = format!("```toml\n{}\n```", saved().to_toml());
        let proposal = Action::Index.proposal(&reply, None).unwrap();
        assert_eq!(proposal, saved());
        assert!(Action::Index.proposal("No idea.", None).is_err());
    }

    #[test]
    fn update_places_only_unsorted_files_and_never_moves_the_rest() {
        let reply = r#"```toml
[[place]]
file = "src/gifts/Gift.ts"
domain = "Gift Cards"

[[place]]
file = "src/gifts/log.ts"
domain = "Shared"

[[place]]
file = "src/events/Event.ts"
domain = "Bookings"

[[new_domain]]
name = "Gift Cards"
purpose = "Give a gift."
offers = ["Gift cards"]
why = "Gifts."

[[new_domain]]
name = "Nothing Placed"

[[new_flow]]
from = "Gift Cards"
to = "Bookings"
label = "pays for"
```"#;
        let unsorted = vec!["src/gifts/Gift.ts".to_string(), "src/gifts/log.ts".into()];
        let proposal = Action::Update(unsorted)
            .proposal(reply, Some(&saved()))
            .unwrap();
        let names: Vec<&str> = proposal.domains.iter().map(|d| d.name.as_str()).collect();
        assert_eq!(names, ["Bookings", "Events", "Gift Cards"]);
        assert_eq!(proposal.home_of("src/gifts/Gift.ts"), Home::Domain(2));
        assert_eq!(proposal.home_of("src/gifts/log.ts"), Home::Shared);
        // Confirmed files stay where they were.
        assert_eq!(proposal.home_of("src/events/Event.ts"), Home::Domain(1));
        assert_eq!(proposal.domains[..2], saved().domains[..]);
        assert_eq!(proposal.flows.len(), 2);
    }

    #[test]
    fn a_split_replaces_the_domain_where_it_was() {
        let reply = r#"```toml
[[domain]]
name = "Reservations"
members = ["src/bookings/reserve/"]

[[domain]]
name = "Refunds"
members = ["src/bookings/refund/"]

[[flow]]
from = "Events"
to = "Reservations"
label = "supplies events to"

[[flow]]
from = "Reservations"
to = "Refunds"
label = "cancels into"
```"#;
        let proposal = Action::Split("Bookings".into())
            .proposal(reply, Some(&saved()))
            .unwrap();
        let names: Vec<&str> = proposal.domains.iter().map(|d| d.name.as_str()).collect();
        assert_eq!(names, ["Reservations", "Refunds", "Events"]);
        assert_eq!(proposal.flows.len(), 2);
        assert!(proposal.flows.iter().all(|f| f.to != "Bookings"));
    }

    #[test]
    fn only_commands_that_read_run_without_asking() {
        for command in [
            "ls -la src",
            "rg -n \"fn main\" crates | head -20",
            "cd src && git ls-files | wc -l",
            "sed -n 1,80p src/main.rs",
            "find . -name '*.rs' | xargs grep -l Booking",
            "ls src/bookings 2>/dev/null",
            "find . -name '*.svelte' 2> /dev/null | head -50",
            "cat package.json >/dev/null 2>&1",
            "rg -n 'book|pay' src &>/dev/null",
            "grep -E \"Booking|Payment\" -r src | sort",
            "rg -n '->' src",
        ] {
            assert!(read_only_command(command), "{command}");
        }
        for command in [
            "rm -rf target",
            "echo hi > notes.txt",
            "sed -i s/a/b/ src/main.rs",
            "find . -name '*.tmp' -delete",
            "git checkout main",
            "cat $(which python)",
            "npm install",
            "ls > files.txt 2>/dev/null",
            "cat a >> /dev/nullish",
            "grep x src 2>errors.log",
            "echo \"$(rm -rf src)\"",
            "diff <(rm -rf src) b",
            "ls; rm -rf src 2>/dev/null",
        ] {
            assert!(!read_only_command(command), "{command}");
        }
    }

    #[test]
    fn requests_are_known_by_what_they_do() {
        let request = |kind: Asking, detail: &str| PermissionRequest {
            id: "1".into(),
            title: "Bash".into(),
            detail: detail.into(),
            kind,
            always: None,
            task: None,
        };
        assert!(only_reads(&request(Asking::Command, "ls 2>/dev/null")));
        assert!(!only_reads(&request(Asking::Command, "rm -rf src")));
        assert!(only_reads(&request(Asking::Read, "src/main.rs")));
        assert!(only_reads(&request(
            Asking::Subagent { explores: true },
            "Find the routes"
        )));
        assert!(!only_reads(&request(
            Asking::Subagent { explores: false },
            "Do it"
        )));
        assert!(!only_reads(&request(Asking::Web, "https://example.com")));
        assert!(!only_reads(&request(Asking::Other, "ls")));
    }
}
