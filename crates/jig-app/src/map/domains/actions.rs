//! The Map's actions on domains: what the agent is asked (Index Domains,
//! Update Domains, splitting a domain), each in a normal conversation the
//! user can watch and steer, and what its answer becomes: a proposal,
//! reviewed on the canvas before it's saved. Jig applies an update itself,
//! so the agent can place unsorted files and add domains but never move
//! what was confirmed.

use anyhow::{Result, bail};

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

    /// The domains its answer `reply` proposes, from `saved`.
    pub fn proposal(&self, reply: &str, saved: Option<&Domains>) -> Result<Domains> {
        let text = fenced_toml(reply);
        match self {
            Action::Index => {
                let mut domains: Domains = toml::from_str(text)?;
                if domains.domains.is_empty() {
                    bail!("The agent's answer has no domains in it.");
                }
                domains.tidy_flows();
                Ok(domains)
            }
            Action::Update(unsorted) => {
                let update: Update = toml::from_str(text)?;
                Ok(update.apply(saved.cloned().unwrap_or_default(), unsorted))
            }
            Action::Split(name) => {
                let split: Domains = toml::from_str(text)?;
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
        }
    }
}

/// Update Domains' answer: where each unsorted file goes, and any new
/// domain or flow it needs.
#[derive(Debug, Default, serde::Deserialize)]
struct Update {
    #[serde(default)]
    place: Vec<Place>,
    #[serde(default)]
    new_domain: Vec<Domain>,
    #[serde(default)]
    new_flow: Vec<Flow>,
}

#[derive(Debug, serde::Deserialize)]
struct Place {
    file: String,
    domain: String,
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

fn is_shared(name: &str) -> bool {
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

/// What Index Domains asks, with Jig's grouping as a hint.
pub fn index_instruction(hint: &str) -> String {
    format!(
        "You are indexing this project's business domains for Jig's Map. Its top level shows the project as a few business areas, for programmers finding their way and for non-engineers (a product manager, a client) who never read code.

Below is Jig's own grouping of the project: every source file, including ones Jig's index can't read, grouped by folder and calls, with the words their function names use most and the calls between groups. It's a starting hint, not an answer: folders often cut across business areas, and the words are guesses.

Do this:
1. Read enough of the code to know what the project does for the people who use it: entry points (routes, controllers, screens, commands, jobs), the main entities, and names in the domain's own language (keep non-English business terms, and explain them).
2. Group the files into 3 to 9 business domains. Each file belongs to one domain; when a file mixes several, name a type instead (`path#Type`). Code used everywhere (configuration, logging, HTTP plumbing, shared utilities, build scripts) goes in `shared`, not in a domain. Every file in the hint must end up in a domain or in `shared`.
3. For each domain write a name a non-engineer understands (two or three words), a purpose of one or two plain sentences with no code terms, what it offers people in plain words (screens, commands, jobs, emails: what someone can do or gets, never routes, URLs, endpoints or code names), and one sentence on why these files belong together.
4. For the arrows between domains, say what flows along each in a few plain words (\"books rooms for\", \"sends invites to\").

Only read; change nothing. Use your read and search tools. Reply with only this TOML, in one fenced block:

{DOMAINS_FORMAT}

{hint}"
    )
}

/// What Update Domains asks: place only the unsorted files.
pub fn update_instruction(unsorted: &[String], saved: &Domains) -> String {
    let files: Vec<String> = unsorted.iter().map(|file| format!("- {file}")).collect();
    format!(
        "You are updating this project's business domains for Jig's Map. They were indexed earlier and confirmed by a person; the saved result is below. Since then these files appeared that no domain holds:

{}

Do this:
1. Read the unsorted files, and as much around them as you need, to know what they do for the people who use the project.
2. Put each in the existing domain it belongs to, or in `Shared` if it's code used everywhere. Only when none fits, propose one new domain (a name a non-engineer understands, a plain purpose, what it offers people in plain words with no routes or code names, why). Never move or rename what is already placed, and don't change existing domains.
3. Say if the new code adds a flow between domains.

Only read; change nothing. Reply with only this TOML, in one fenced block:

```toml
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
```

The saved domains:

{}",
        files.join("\n"),
        saved.to_toml()
    )
}

/// What splitting domain `name` asks.
pub fn split_instruction(name: &str, saved: &Domains) -> String {
    format!(
        "You are splitting one of this project's business domains for Jig's Map: \"{name}\" holds too much. The saved domains are below; the others were confirmed by a person and stay as they are.

Do this:
1. Read the files of \"{name}\", as much as you need, to see the business areas in it.
2. Split it into two or more domains, each with a name a non-engineer understands (two or three words), a purpose of one or two plain sentences with no code terms, what it offers people in plain words (no routes or code names), why its files belong together, and its members. Every member of \"{name}\" must end up in one of them. Don't reuse another domain's name.
3. Give the flows between the new domains, and between them and the other domains, each in a few plain words.

Only read; change nothing. Reply with only this TOML, in one fenced block, holding just the new domains and their flows:

{DOMAINS_FORMAT}

The saved domains:

{}",
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
    if command.contains('>') || command.contains('`') || command.contains("$(") {
        return false;
    }
    command
        .split(['|', ';', '&', '\n'])
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .all(|part| {
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
        ] {
            assert!(!read_only_command(command), "{command}");
        }
    }
}
