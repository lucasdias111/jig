//! The Map's actions on domains done with the Map's tools rather than
//! shell commands, so nothing waits for the user's go-ahead.
//! `map_project` hands the agent what it would otherwise dig out of the
//! project: Jig's grouping, every source file (those the index can't read
//! too), each file's purpose, types, functions and imports, and the calls
//! between groups, a page at a time. `map_propose_domains` takes the
//! answer as typed fields, checked against the project, so a file or
//! domain the agent got wrong comes back as an error it can fix rather
//! than a proposal that can't be read.

use std::collections::HashMap;

use serde_json::{Value, json};

use super::actions::{Action, Answer, Place, Update, is_shared};
use super::grouping::Grouping;
use super::{Domain, Domains, Flow, Home, Shared, place_of};
use crate::index::{Project, Target};
use crate::plan::signature;

/// Files described in full on one page of `map_project`.
const DETAIL_PAGE: usize = 40;
/// Paths on one page of `map_project` with `files_only`.
const PATHS_PAGE: usize = 500;
/// More files than this, and the overview lists groups without them.
const OVERVIEW_FILES: usize = 600;
/// A file's functions listed before "+N more".
const FUNCTIONS_SHOWN: usize = 30;
/// Unplaced files named in an error before "and N more".
const NAMED: usize = 40;

/// Every source file as domains name them.
pub fn source_files(project: &Project) -> Vec<String> {
    super::all_files(project)
        .iter()
        .map(|path| place_of(path))
        .collect()
}

/// `map_project`'s answer for `input`: the overview, a group's or a
/// folder's files in detail, or just paths, a page at a time.
pub fn project_outline(
    project: &Project,
    domains: Option<&Domains>,
    input: &Value,
) -> Result<String, String> {
    let grouping = Grouping::of(project);
    let files = source_files(project);
    let page = input["page"].as_u64().unwrap_or(1).max(1) as usize;
    let group = input["group"].as_u64().map(|group| group as usize);
    let folder = input["folder"]
        .as_str()
        .map(|folder| folder.trim().trim_start_matches("./").to_string())
        .filter(|folder| !folder.is_empty() && folder != "." && folder != "/");
    let files_only = input["files_only"].as_bool().unwrap_or(false);

    let scope: Vec<String> = match (group, &folder) {
        (Some(group), _) => {
            let Some(found) = group.checked_sub(1).and_then(|ix| grouping.groups.get(ix)) else {
                return Err(format!(
                    "There's no group {group}: the groups are numbered 1 to {}.",
                    grouping.groups.len()
                ));
            };
            found.files.iter().map(|path| place_of(path)).collect()
        }
        (None, Some(folder)) => {
            let prefix = format!("{}/", folder.trim_end_matches('/'));
            let held: Vec<String> = files
                .iter()
                .filter(|file| file.starts_with(&prefix) || *file == folder)
                .cloned()
                .collect();
            if held.is_empty() {
                return Err(format!(
                    "No source file is in {folder}. Name a folder relative to the project, as map_project's overview lists them."
                ));
            }
            held
        }
        (None, None) if files_only => files.clone(),
        (None, None) => return Ok(overview(project, &grouping, domains, &files)),
    };
    let size = if files_only { PATHS_PAGE } else { DETAIL_PAGE };
    let pages = scope.len().div_ceil(size).max(1);
    if page > pages {
        return Err(format!("There are only {pages} pages."));
    }
    let shown = &scope[(page - 1) * size..(page * size).min(scope.len())];
    let mut out = String::new();
    if files_only {
        for file in shown {
            out.push_str(&format!("- {file}\n"));
        }
    } else {
        let group_of = group_of(&grouping);
        for file in shown {
            out.push_str(&describe(
                project,
                file,
                group_of.get(file.as_str()).copied(),
            ));
        }
    }
    if pages > 1 {
        out.push_str(&format!(
            "\nPage {page} of {pages} ({} files).{}\n",
            scope.len(),
            if page < pages {
                format!(" Call again with \"page\": {} for more.", page + 1)
            } else {
                String::new()
            }
        ));
    }
    Ok(out)
}

/// What `map_project` says with nothing narrowed: the grouping, every
/// file by group while there aren't too many, and the calls between
/// groups.
fn overview(
    project: &Project,
    grouping: &Grouping,
    domains: Option<&Domains>,
    files: &[String],
) -> String {
    let unread = project.others.len();
    let mut out = format!(
        "{} source files: {} read by Jig's index, {unread} it can't read (pages, templates, other languages; read those with your own tools). Every one of them is in a group below.\n",
        files.len(),
        files.len() - unread
    );
    if let Some(domains) = domains {
        let names: Vec<&str> = domains.domains.iter().map(|d| d.name.as_str()).collect();
        let unsorted = files
            .iter()
            .filter(|file| domains.home_of(file) == Home::Unsorted)
            .count();
        out.push_str(&format!(
            "Saved domains: {}; {unsorted} files in none of them.\n",
            names.join(", ")
        ));
    }
    out.push('\n');
    out.push_str(&grouping_hint(grouping));
    out.push_str(
        "\nFor each file's purpose, types, functions, imports and calls, call map_project with {\"group\": N} or {\"folder\": \"path/\"}; for just the paths, {\"files_only\": true}. Long answers come in pages: add {\"page\": 2}.\n",
    );
    out
}

/// Jig's grouping as the agent is handed it: every file by group, or
/// for a big project just the groups, and the calls between them.
pub fn grouping_hint(grouping: &Grouping) -> String {
    let files: usize = grouping.groups.iter().map(|group| group.files.len()).sum();
    if files <= OVERVIEW_FILES {
        return grouping.hint();
    }
    let mut out =
        String::from("# Jig's grouping (folders, calls and names; the words are guesses)\n");
    for (ix, group) in grouping.groups.iter().enumerate() {
        out.push_str(&format!(
            "- group {} — words: {} — {} · {} files, {} functions\n",
            ix + 1,
            group.words,
            group.folder,
            group.files.len(),
            group.functions
        ));
    }
    out.push_str("\nThere are too many files to list here: map_project lists each group's.\n");
    if !grouping.calls.is_empty() {
        out.push_str("\n## calls between groups (from → to: count)\n");
        for ((from, to), count) in &grouping.calls {
            out.push_str(&format!(
                "- group {} → group {}: {count}\n",
                from + 1,
                to + 1
            ));
        }
    }
    out
}

/// Each file's group, numbered from 1.
fn group_of(grouping: &Grouping) -> HashMap<String, usize> {
    grouping
        .groups
        .iter()
        .enumerate()
        .flat_map(|(ix, group)| group.files.iter().map(move |file| (place_of(file), ix + 1)))
        .collect()
}

/// One file, as `map_project` describes it.
fn describe(project: &Project, place: &str, group: Option<usize>) -> String {
    let group = group.map(|g| format!(" (group {g})")).unwrap_or_default();
    let Some(ix) = project
        .files
        .iter()
        .position(|file| place_of(&file.path) == place)
    else {
        return format!(
            "\n### {place}{group}\nNot read by Jig's index: read it yourself if its name doesn't say enough.\n"
        );
    };
    let file = &project.files[ix];
    let mut out = format!("\n### {place}{group}\n");
    if let Some(purpose) = &file.purpose {
        out.push_str(&format!("purpose: {purpose}\n"));
    }
    if !file.types.is_empty() {
        let types: Vec<String> = file
            .types
            .iter()
            .map(|ty| {
                let mut line = format!("{} {}", ty.kind, ty.name);
                if !ty.implements.is_empty() {
                    line.push_str(&format!(" ({})", ty.implements.join(", ")));
                }
                if let Some(purpose) = &ty.purpose {
                    line.push_str(&format!(": {purpose}"));
                }
                line
            })
            .collect();
        out.push_str(&format!("types: {}\n", types.join("; ")));
    }
    let functions: Vec<usize> = (0..project.functions.len())
        .filter(|&id| project.functions[id].file == ix)
        .collect();
    if !functions.is_empty() {
        let mut named: Vec<String> = functions
            .iter()
            .take(FUNCTIONS_SHOWN)
            .map(|&id| {
                let function = &project.functions[id];
                format!("{}{}", function.title(), signature(function))
            })
            .collect();
        if functions.len() > FUNCTIONS_SHOWN {
            named.push(format!("+{} more", functions.len() - FUNCTIONS_SHOWN));
        }
        out.push_str(&format!("functions: {}\n", named.join("; ")));
    }
    if !file.imports.is_empty() {
        let imports: Vec<&str> = file
            .imports
            .iter()
            .map(|import| import.path.as_str())
            .collect();
        out.push_str(&format!("imports: {}\n", imports.join(", ")));
    }
    let mut calls: HashMap<usize, usize> = HashMap::new();
    for &id in &functions {
        for edge in &project.edges[id] {
            if let Target::Function(to) = edge.target {
                let into = project.functions[to].file;
                if into != ix {
                    *calls.entry(into).or_default() += 1;
                }
            }
        }
    }
    if !calls.is_empty() {
        let mut calls: Vec<(usize, usize)> = calls.into_iter().collect();
        calls.sort_by_key(|&(file, count)| (std::cmp::Reverse(count), file));
        let calls: Vec<String> = calls
            .iter()
            .take(8)
            .map(|&(file, count)| format!("{} ({count})", place_of(&project.files[file].path)))
            .collect();
        out.push_str(&format!("calls into: {}\n", calls.join(", ")));
    }
    out
}

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
pub(super) fn answer(action: &Action, input: &Value) -> Result<Answer, String> {
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
pub(super) fn check(
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
fn check_names(domains: &[Domain], taken: &[&str], problems: &mut Vec<String>) -> Vec<String> {
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

fn check_flows(flows: &[Flow], known: &dyn Fn(&str) -> bool, problems: &mut Vec<String>) {
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
fn is_real(member: &str, files: &[String]) -> bool {
    let member = member.trim().trim_start_matches("./");
    let file = member.split('#').next().unwrap_or(member);
    if member.ends_with('/') {
        return files.iter().any(|f| f.starts_with(member));
    }
    files.iter().any(|f| f == file)
}

/// An unknown member, with files of its name that are real.
fn unknown_member(member: &str, files: &[String]) -> String {
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

fn named(files: &[&String]) -> String {
    let mut out: Vec<&str> = files.iter().take(NAMED).map(|f| f.as_str()).collect();
    let more = files.len().saturating_sub(NAMED);
    let more = format!("and {more} more");
    if files.len() > NAMED {
        out.push(&more);
    }
    out.join(", ")
}

/// The two tools' schemas, as `tools/list` gives them.
pub fn schemas() -> Vec<Value> {
    let text = |what: &str| json!({"type": "string", "description": what});
    let strings =
        |what: &str| json!({"type": "array", "items": {"type": "string"}, "description": what});
    let domain = json!({"type": "object", "properties": {
        "name": text("Two or three words a non-engineer understands."),
        "purpose": text("One or two plain sentences, no code terms."),
        "offers": strings("What people can do or get from it, in plain words: screens, commands, jobs, emails. Never routes, URLs, endpoints or code names."),
        "why": text("One sentence on why these files belong together."),
        "members": strings("Files (path/to/file.ts), types of their own (path/to/File.java#Type) or folders (path/to/folder/, every file under it), relative to the project.")
    }, "required": ["name", "purpose", "offers", "why", "members"]});
    let flow = json!({"type": "object", "properties": {
        "from": text("A domain's name."),
        "to": text("A domain's name."),
        "label": text("What flows along it, in a few plain words: \"books rooms for\".")
    }, "required": ["from", "to", "label"]});
    let read_only = json!({"readOnlyHint": true, "destructiveHint": false});
    vec![
        json!({
            "name": jig_ai::map::PROJECT,
            "title": "Read the project's outline",
            "description": "Jig's outline of the project, from its index: with no arguments, its draft grouping with every source file (those the index can't read too) by group and the calls between groups; with \"group\" or \"folder\", each file's purpose, types, functions with signatures, imports and the files it calls into; with \"files_only\", just the paths. Long answers come in pages.",
            "inputSchema": {"type": "object", "properties": {
                "group": {"type": "integer", "description": "A group's number from the overview."},
                "folder": text("A folder (or file) relative to the project."),
                "files_only": {"type": "boolean", "description": "Just the paths."},
                "page": {"type": "integer", "description": "Which page, from 1."}
            }},
            "annotations": read_only,
        }),
        json!({
            "name": jig_ai::map::PROPOSE_DOMAINS,
            "title": "Propose domains",
            "description": "Answer Index Domains, Update Domains or a split with the domains you propose; they show on the Map as a proposal the user keeps or discards. Index and split: \"domains\", \"flows\" and \"shared\". Update: \"placements\", \"new_domains\" and \"new_flows\". It's checked against the project: unknown files or domain names, or files left without a home, come back as an error to fix; then call it again with the whole answer.",
            "inputSchema": {"type": "object", "properties": {
                "action": {"type": "string", "enum": ["index", "update", "split"], "description": "Which action this answers."},
                "domain": text("For a split: the domain being split."),
                "domains": {"type": "array", "items": domain, "description": "Index and split: the domains."},
                "flows": {"type": "array", "items": flow, "description": "Index and split: what flows between domains."},
                "shared": strings("Index and split: code used everywhere (configuration, logging, plumbing, utilities, build scripts), as members."),
                "placements": {"type": "array", "items": {"type": "object", "properties": {
                    "file": text("An unsorted file."),
                    "domain": text("A saved or new domain's name, or Shared.")
                }, "required": ["file", "domain"]}, "description": "Update: where each unsorted file goes."},
                "new_domains": {"type": "array", "items": domain, "description": "Update: only when no saved domain fits; its members come from the placements."},
                "new_flows": {"type": "array", "items": flow, "description": "Update: flows the new code adds."}
            }, "required": ["action"]},
            "annotations": read_only,
        }),
    ]
}

#[cfg(test)]
mod tests;
