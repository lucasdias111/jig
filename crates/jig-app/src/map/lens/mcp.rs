//! `Jig --map-mcp [--project <folder>]`: the Map's tools as an MCP server
//! on stdio, beside the task tool. Each call is checked against the
//! project's index (read in the background as the server starts, and
//! again, file by file, for the tools of the Map's actions on domains)
//! and answered with what was drawn or the names it couldn't find with
//! names like them, so the agent can correct itself. The drawing, or the
//! domains proposed, reach the Map from the agent's own report of the
//! call. `map_project` and `map_propose_domains` (`domains::tools`) let
//! Index Domains and the rest run without a shell.

use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc;

use anyhow::Result;
use serde_json::{Value, json};

use super::super::domains::{Domains, tools as domain_tools};
use super::super::index::Project;
use super::{Call, check};

/// How the agent should draw, sent as the server's instructions.
pub const INSTRUCTIONS: &str = "These tools draw on Jig's Map, the zoomable picture of the project the user is looking at (its levels: Domain, the business areas; Site, components, folders and files; Plan, functions and their calls). Drawing changes no code. When the user asks how something works, where something happens or what to change, read the code first so every place you name is real, then answer by drawing: map_trace for a flow in order, each step labelled with what happens there in a few plain words (map_tour for one worth flying through step by step); map_highlight for the places that matter; map_note for a remark on one place; map_group to circle places that belong together; map_propose for new boxes and arrows you suggest, as ghosts; map_describe to propose purposes for boxes or labels for arrows. Then reply with a short text, a few sentences, that refers to what you drew, writing each place you mention as a Markdown link to it, e.g. [BookingService.create](src/bookings/BookingService.java#BookingService.create), so a click flies the Map there. Name places relative to the project as path/to/file.ext#Type.method for a method, path/to/file.ext#function for a free function, path/to/file.ext for a file (any file, a Svelte page or template too), path/to/folder/ for a folder, or domain:Name for a domain. When the user asks about a sketch of theirs (boxes, arrows and notes they drew), it's theirs: never redraw or change it; answer beside it, with map_note on its boxes (named sketch:<sketch>/<box>) or real places, and map_propose with boxes near its boxes for what you'd add or change. A name that isn't found is reported back with names like it, and nothing is drawn: fix it and draw again. map_clear removes what you drew in this conversation. For Jig's actions on domains (Index Domains, Update Domains, splitting a domain), map_project gives Jig's outline of the project and map_propose_domains takes the answer, so no shell command is needed.";

/// A place, as each tool's schema describes it.
const PLACE: &str = "path/to/file.ext#Type.method, path/to/file.ext#function, path/to/file.ext, path/to/folder/, domain:Name, or sketch:<sketch>/<box> for a box of the user's sketch, relative to the project";

/// Answer requests from `input` on `output` until the agent hangs up.
pub fn serve(root: &Path, input: impl BufRead, mut output: impl Write) -> Result<()> {
    // Read the project while the agent starts up; the first call waits.
    let (sender, read) = mpsc::channel();
    let at = root.to_path_buf();
    std::thread::spawn(move || {
        let _ = sender.send(Project::read_again(&at, None));
    });
    let mut project: Option<Project> = None;
    for line in input.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let Ok(message) = serde_json::from_str::<Value>(&line) else {
            let error = json!({"jsonrpc": "2.0", "id": null, "error": {"code": -32700, "message": "Parse error"}});
            writeln!(output, "{error}")?;
            output.flush()?;
            continue;
        };
        let Some(id) = message.get("id").cloned() else {
            continue;
        };
        let method = message["method"].as_str().unwrap_or_default();
        let params = &message["params"];
        let result = match method {
            "initialize" => Ok(json!({
                "protocolVersion": params["protocolVersion"].as_str().unwrap_or("2025-06-18"),
                "capabilities": {"tools": {}},
                "serverInfo": {"name": jig_ai::map::NAME, "title": "Jig Map", "version": env!("CARGO_PKG_VERSION")},
                "instructions": INSTRUCTIONS,
            })),
            "ping" => Ok(json!({})),
            "tools/list" => Ok(json!({"tools": tools()})),
            "tools/call" => {
                if project.is_none() {
                    project = read.recv().ok();
                }
                let name = params["name"].as_str().unwrap_or_default();
                // The domains tools see the project as it is now.
                if [jig_ai::map::PROJECT, jig_ai::map::PROPOSE_DOMAINS].contains(&name)
                    && let Some(before) = &project
                {
                    project = Some(Project::read_again(root, Some(before)));
                }
                let domains = Domains::read(root);
                let answer = match &project {
                    Some(project) => answer(project, domains.as_ref(), name, &params["arguments"]),
                    None => Err("The project couldn't be read.".into()),
                };
                Ok(match answer {
                    Ok(text) => json!({"content": [{"type": "text", "text": text}]}),
                    Err(text) => {
                        json!({"content": [{"type": "text", "text": text}], "isError": true})
                    }
                })
            }
            _ => Err(-32601),
        };
        let reply = match result {
            Ok(result) => json!({"jsonrpc": "2.0", "id": id, "result": result}),
            Err(code) => {
                json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": format!("Unknown method {method}")}})
            }
        };
        writeln!(output, "{reply}")?;
        output.flush()?;
    }
    Ok(())
}

/// A call checked: what was drawn, or why nothing was.
fn answer(
    project: &Project,
    domains: Option<&Domains>,
    tool: &str,
    arguments: &Value,
) -> Result<String, String> {
    if tool == jig_ai::map::PROJECT {
        return domain_tools::project_outline(project, domains, arguments);
    }
    if tool == jig_ai::map::PROPOSE_DOMAINS {
        let files = domain_tools::source_files(project);
        let action = domain_tools::action_of(arguments, domains, &files)?;
        let proposal = action.proposal_from_call(arguments, domains, &files)?;
        return Ok(format!(
            "Proposed {} domains on the Map. The user keeps, edits or discards them there; reply in a sentence or two.",
            proposal.domains.len()
        ));
    }
    match super::call(tool, arguments)? {
        Call::Clear => Ok("Cleared what you drew on the Map.".into()),
        Call::Draw(drawing) => {
            check(project, domains, &drawing)?;
            Ok(format!(
                "Drawn on the Map: {}. The user sees it on whichever level they're at.",
                drawing.summary()
            ))
        }
    }
}

/// The project folder: `--project <folder>`, else where it was started.
pub fn project_from_args(args: &[String]) -> Result<PathBuf> {
    jig_ai::tasks::mcp::project_from_args(args)
}

fn tools() -> Value {
    let place = json!({"type": "string", "description": PLACE});
    let places = json!({"type": "array", "items": place});
    let arrows = json!({"type": "array", "items": {"type": "object", "properties": {
        "from": place, "to": place, "label": {"type": "string"}
    }, "required": ["from", "to"]}});
    let steps = json!({"type": "array", "items": {"type": "object", "properties": {
        "place": place,
        "label": {"type": "string", "description": "What happens here, in a few plain words."}
    }, "required": ["place", "label"]}});
    let title = |what: &str| json!({"type": "string", "description": what});
    let tool = |name: &str, title: &str, description: &str, schema: Value| {
        json!({
            "name": name,
            "title": title,
            "description": description,
            "inputSchema": schema,
            "annotations": {"readOnlyHint": true, "destructiveHint": false},
        })
    };
    let mut tools = vec![
        tool(
            "map_trace",
            "Trace a flow",
            "Draw a flow through the project on the Map, step by step in order, each step a place with what happens there. At coarse levels the steps fold onto the boxes holding them.",
            json!({"type": "object", "properties": {"title": title("What the flow is, in a few words."), "steps": steps}, "required": ["title", "steps"]}),
        ),
        tool(
            "map_tour",
            "Draw a tour",
            "A trace the user can fly through step by step with the arrow keys: for a flow worth following closely.",
            json!({"type": "object", "properties": {"title": title("What the tour shows, in a few words."), "steps": steps}, "required": ["title", "steps"]}),
        ),
        tool(
            "map_highlight",
            "Highlight places",
            "Light up places on the Map that matter to the answer, and optionally arrows between them.",
            json!({"type": "object", "properties": {"title": title("What they have in common."), "places": places, "arrows": arrows}, "required": ["title", "places"]}),
        ),
        tool(
            "map_note",
            "Pin a note",
            "Pin a short note to one place on the Map.",
            json!({"type": "object", "properties": {"place": place, "text": title("The note, a sentence or two.")}, "required": ["place", "text"]}),
        ),
        tool(
            "map_group",
            "Group places",
            "Circle places that belong together, with a label.",
            json!({"type": "object", "properties": {"title": title("The group's label."), "places": places}, "required": ["title", "places"]}),
        ),
        tool(
            "map_propose",
            "Propose a change",
            "Draw new boxes and arrows you suggest as ghosts beside the real ones: never code. The user can turn it into a task or have it implemented.",
            json!({"type": "object", "properties": {
                "title": title("The change, in a few words."),
                "boxes": {"type": "array", "items": {"type": "object", "properties": {
                    "name": title("The new box's name, e.g. a type or file."),
                    "near": place,
                    "purpose": title("What it would do.")
                }, "required": ["name", "near"]}},
                "arrows": {"type": "array", "items": {"type": "object", "properties": {
                    "from": title("A new box's name, or a place."), "to": title("A new box's name, or a place."), "label": {"type": "string"}
                }, "required": ["from", "to"]}}
            }, "required": ["title", "boxes"]}),
        ),
        tool(
            "map_describe",
            "Describe boxes",
            "Propose a one-line purpose for boxes, or a label for the arrow between two places. They show as proposals the user keeps or rejects.",
            json!({"type": "object", "properties": {
                "purposes": {"type": "array", "items": {"type": "object", "properties": {
                    "place": place, "purpose": title("What it's for, one plain line.")
                }, "required": ["place", "purpose"]}},
                "labels": arrows
            }}),
        ),
        tool(
            "map_clear",
            "Clear the Map",
            "Remove everything drawn on the Map in this conversation.",
            json!({"type": "object", "properties": {}}),
        ),
    ];
    tools.extend(domain_tools::schemas());
    Value::Array(tools)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::index::tests::fixture;

    fn exchange(root: &Path, requests: &[Value]) -> Vec<Value> {
        let input: String = requests.iter().map(|r| format!("{r}\n")).collect();
        let mut output = Vec::new();
        serve(root, input.as_bytes(), &mut output).unwrap();
        String::from_utf8(output)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }

    fn call_tool(id: u64, name: &str, arguments: Value) -> Value {
        json!({"jsonrpc": "2.0", "id": id, "method": "tools/call", "params": {"name": name, "arguments": arguments}})
    }

    #[test]
    fn draws_what_resolves_and_suggests_what_doesnt() {
        let dir = fixture();
        let answers = exchange(
            dir.path(),
            &[
                json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-03-26"}}),
                json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}),
                call_tool(
                    3,
                    "map_trace",
                    json!({"title": "Finding a user", "steps": [
                        {"place": "server/src/org/demo/bugs/BugService.java#BugService.create", "label": "Asks"},
                        {"place": "server/src/org/demo/users/UserService.java#UserService.find", "label": "Finds"},
                    ]}),
                ),
                call_tool(
                    4,
                    "map_highlight",
                    json!({"title": "x", "places": ["server/src/org/demo/users/UserService.java#UserService.remove"]}),
                ),
            ],
        );
        assert!(
            answers[0]["result"]["instructions"]
                .as_str()
                .unwrap()
                .contains("Markdown link")
        );
        let names: Vec<&str> = answers[1]["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|tool| tool["name"].as_str().unwrap())
            .collect();
        let mut all = jig_ai::map::TOOLS.to_vec();
        let mut listed = names.clone();
        all.sort();
        listed.sort();
        assert_eq!(listed, all);
        assert_eq!(answers[2]["result"]["isError"], Value::Null);
        let text = answers[3]["result"]["content"][0]["text"].as_str().unwrap();
        assert_eq!(answers[3]["result"]["isError"], true);
        assert!(text.contains("UserService.find"), "{text}");
    }
}
