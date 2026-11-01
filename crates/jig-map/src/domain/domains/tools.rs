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
use crate::domain::index::{Project, Target};
use crate::domain::plan::signature;
mod checking;
mod outline;
pub use checking::*;
pub use outline::*;

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
