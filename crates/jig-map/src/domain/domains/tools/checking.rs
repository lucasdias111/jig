//! `map_propose_domains`: the agent's answer read and checked against the project.

use super::*;

/// Which action a call of `map_propose_domains` answers, from its
/// `action` field: for an update, the files no saved domain holds.
pub fn action_of(
    input: &Value,
    saved: Option<&Domains>,
    files: &[String],
) -> Result<Action, String> {
    match input["action"].as_str().unwrap_or_default() {
        "index" => Ok(Action::Index),
        "update" => {
            let Some(saved) = saved else {
                return Err(
                    "There are no saved domains to update: use \"action\": \"index\".".into(),
                );
            };
            Ok(Action::Update(
                files
                    .iter()
                    .filter(|file| saved.home_of(file) == Home::Unsorted)
                    .cloned()
                    .collect(),
            ))
        }
        "split" => {
            let name = input["domain"].as_str().unwrap_or_default().trim();
            match saved.and_then(|saved| saved.index_of(name)) {
                Some(_) => Ok(Action::Split(name.to_string())),
                None => Err(format!(
                    "\"domain\" must name the saved domain being split; {name:?} isn't one."
                )),
            }
        }
        other => Err(format!(
            "\"action\" must be \"index\", \"update\" or \"split\", not {other:?}."
        )),
    }
}

/// The answer in a call's `input`, as `action` reads it.
pub(in crate::domain::domains) fn answer(action: &Action, input: &Value) -> Result<Answer, String> {
    fn list<T: serde::de::DeserializeOwned>(input: &Value, field: &str) -> Result<Vec<T>, String> {
        match &input[field] {
            Value::Null => Ok(Vec::new()),
            value => serde_json::from_value(value.clone())
                .map_err(|error| format!("\"{field}\" isn't as the schema says: {error}.")),
        }
    }
    match action {
        Action::Update(_) => Ok(Answer::Update(Update {
            place: list::<Place>(input, "placements")?,
            new_domain: list(input, "new_domains")?,
            new_flow: list(input, "new_flows")?,
        })),
        Action::Index | Action::Split(_) => {
            let domains: Vec<Domain> = list(input, "domains")?;
            if domains.is_empty() {
                return Err("\"domains\" is empty: propose the domains.".into());
            }
            let shared = match &input["shared"] {
                Value::Object(_) => list(&input["shared"], "members")?,
                _ => list(input, "shared")?,
            };
            Ok(Answer::Domains(Domains {
                domains,
                flows: list(input, "flows")?,
                shared: Shared { members: shared },
            }))
        }
    }
}

/// Whether `answer` to `action` holds up against `saved` and `files`:
/// every member a real file, folder or type, every name a domain, and
/// every file that needs a home given one. What's wrong is said all at
/// once, so the agent can fix it in one go.
pub(in crate::domain::domains) fn check(
    action: &Action,
    answer: &Answer,
    saved: Option<&Domains>,
    files: &[String],
) -> Result<(), String> {
    let mut problems = Vec::new();
    match (action, answer) {
        (Action::Index | Action::Split(_), Answer::Domains(proposed)) => {
            let others: Vec<&str> = match action {
                Action::Split(name) => saved
                    .map(|saved| {
                        saved
                            .domains
                            .iter()
                            .map(|d| d.name.as_str())
                            .filter(|other| other != name)
                            .collect()
                    })
                    .unwrap_or_default(),
                _ => Vec::new(),
            };
            let names = check_names(&proposed.domains, &others, &mut problems);
            let mut unknown = Vec::new();
            for member in proposed
                .domains
                .iter()
                .flat_map(|d| &d.members)
                .chain(&proposed.shared.members)
            {
                if !is_real(member, files) {
                    unknown.push(unknown_member(member, files));
                }
            }
            if !unknown.is_empty() {
                problems.push(format!(
                    "These members aren't files, folders (ending in /) or path#Type in the project: {}.",
                    unknown.join("; ")
                ));
            }
            let known = |name: &str| names.iter().any(|n| n == name) || others.contains(&name);
            check_flows(&proposed.flows, &known, &mut problems);
            // Index places every file; a split, every file of the domain.
            let needing: Vec<&String> = match action {
                Action::Split(name) => {
                    let ix = saved.and_then(|saved| saved.index_of(name));
                    files
                        .iter()
                        .filter(|file| {
                            saved
                                .zip(ix)
                                .is_some_and(|(saved, ix)| saved.home_of(file) == Home::Domain(ix))
                        })
                        .collect()
                }
                _ => files.iter().collect(),
            };
            let unplaced: Vec<&String> = needing
                .into_iter()
                .filter(|file| proposed.home_of(file) == Home::Unsorted)
                .collect();
            if !unplaced.is_empty() {
                problems.push(format!(
                    "Every file needs a domain or shared; these have none: {}.",
                    named(&unplaced)
                ));
            }
        }
        (Action::Update(unsorted), Answer::Update(update)) => {
            let saved_names: Vec<&str> = saved
                .map(|saved| saved.domains.iter().map(|d| d.name.as_str()).collect())
                .unwrap_or_default();
            let names = check_names(&update.new_domain, &saved_names, &mut problems);
            let known = |name: &str| names.iter().any(|n| n == name) || saved_names.contains(&name);
            let mut strays = Vec::new();
            let mut nowhere = Vec::new();
            for place in &update.place {
                let file = place.file.trim().trim_start_matches("./");
                if !unsorted.iter().any(|u| u == file) {
                    strays.push(file.to_string());
                }
                let domain = place.domain.trim();
                if !is_shared(domain) && !known(domain) {
                    nowhere.push(format!("{file} → {domain}"));
                }
            }
            if !strays.is_empty() {
                problems.push(format!(
                    "Only the unsorted files are placed; these aren't among them: {}.",
                    strays.join(", ")
                ));
            }
            if !nowhere.is_empty() {
                problems.push(format!(
                    "These go to no saved or new domain (or Shared): {}.",
                    nowhere.join(", ")
                ));
            }
            check_flows(&update.new_flow, &known, &mut problems);
            let unplaced: Vec<&String> = unsorted
                .iter()
                .filter(|file| {
                    !update
                        .place
                        .iter()
                        .any(|place| place.file.trim().trim_start_matches("./") == file.as_str())
                })
                .collect();
            if !unplaced.is_empty() {
                problems.push(format!(
                    "Every unsorted file needs a placement; these have none: {}.",
                    named(&unplaced)
                ));
            }
        }
        _ => problems.push("The answer doesn't fit the action.".into()),
    }
    if problems.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "Nothing was proposed: {} Fix these and call map_propose_domains again with the whole answer.",
            problems.join(" ")
        ))
    }
}

/// The domains' names, checked: none empty, none twice, none taken by
/// `taken`, and none called Shared.
pub(super) fn check_names(
    domains: &[Domain],
    taken: &[&str],
    problems: &mut Vec<String>,
) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    for domain in domains {
        let name = domain.name.trim();
        if name.is_empty() {
            problems.push("A domain has no name.".into());
        } else if is_shared(name) || name.eq_ignore_ascii_case("unsorted") {
            problems.push(format!(
                "\"{name}\" can't be a domain's name: put shared code in \"shared\"."
            ));
        } else if names.iter().any(|n| n == name) || taken.contains(&name) {
            problems.push(format!("\"{name}\" names more than one domain."));
        } else {
            names.push(name.to_string());
        }
    }
    names
}

pub(super) fn check_flows(
    flows: &[Flow],
    known: &dyn Fn(&str) -> bool,
    problems: &mut Vec<String>,
) {
    let unknown: Vec<String> = flows
        .iter()
        .flat_map(|flow| [&flow.from, &flow.to])
        .map(|name| name.trim())
        .filter(|name| !known(name))
        .map(str::to_string)
        .collect();
    if !unknown.is_empty() {
        problems.push(format!(
            "These flows name no domain: {}. Name domains as you named them.",
            unknown.join(", ")
        ));
    }
}

/// Whether a member names something real: a file, `path#Type` in one,
/// or a folder holding one.
pub(super) fn is_real(member: &str, files: &[String]) -> bool {
    let member = member.trim().trim_start_matches("./");
    let file = member.split('#').next().unwrap_or(member);
    if member.ends_with('/') {
        return files.iter().any(|f| f.starts_with(member));
    }
    files.iter().any(|f| f == file)
}

/// An unknown member, with files of its name that are real.
pub(super) fn unknown_member(member: &str, files: &[String]) -> String {
    let name = member
        .trim_end_matches('/')
        .split('#')
        .next()
        .unwrap_or(member)
        .rsplit('/')
        .next()
        .unwrap_or(member);
    let like: Vec<&str> = files
        .iter()
        .filter(|file| !name.is_empty() && file.rsplit('/').next() == Some(name))
        .take(3)
        .map(String::as_str)
        .collect();
    if like.is_empty() {
        member.to_string()
    } else {
        format!("{member} (did you mean {})", like.join(", "))
    }
}

pub(super) fn named(files: &[&String]) -> String {
    let mut out: Vec<&str> = files.iter().take(NAMED).map(|f| f.as_str()).collect();
    let more = files.len().saturating_sub(NAMED);
    let more = format!("and {more} more");
    if files.len() > NAMED {
        out.push(&more);
    }
    out.join(", ")
}
