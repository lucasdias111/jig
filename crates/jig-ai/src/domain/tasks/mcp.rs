//! The task planner as a tool for agents: Jig's own executable, started
//! with [`FLAG`], is a small MCP server on stdio that creates, lists and
//! updates the project's tasks. OpenCode gets it in its config and starts
//! it in the project's folder; ACP agents get it with each conversation,
//! told the folder. Either way it's one process per project, writing the
//! same files the sidebar shows.

use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};
use serde_json::{Value, json};

use super::{TaskStore, Todo};

/// Run Jig as the task tool: `Jig --tasks-mcp [--project <folder>]`.
pub const FLAG: &str = "--tasks-mcp";

/// The server's name; OpenCode calls its tools `jig_create_task` and so on.
pub const NAME: &str = "jig";

/// The tools by name, for agents that want them listed.
pub const TOOLS: [&str; 3] = [CREATE, "list_tasks", "update_task"];

/// The tool that saves a plan as a task, which the user is asked about.
pub const CREATE: &str = "create_task";

/// How agents should go about tasks, sent as the server's instructions.
const INSTRUCTIONS: &str = "Jig's task planner: tasks with a title, a description and a todo list, which the user keeps in Jig's sidebar and works through. Write a plan as a numbered list, one step per item. Whenever you propose three or more steps, or work that spans several files, call create_task with the plan instead of asking in your reply whether to save it: Jig asks the user, who saves it, turns it down, or answers with something to change, which you should follow. Don't propose the same plan again once it's turned down. As you finish a todo of a task you're working on, check it off with update_task.";

/// What a task holds, and no more, so it reads at a glance in the sidebar.
const SHAPE: &str = "Keep a task short and concise. Its description is a one or two sentence summary, then exactly these four sections, each a sentence or two: `### Background` (how things are now), `### Context` (what else it touches), `### Purpose` (why it's worth doing) and `### Risks` (what could go wrong). Nothing else: no rules, file lists or deferred work. The steps go in the todos, one short line each.";

/// How to refer to code in a task.
const LINKS: &str = "Write every reference to code (functions, types, variables, files) as a Markdown link with the name as its text and the path relative to the project, with the line, as its target: [`parse_reply`](crates/jig-ai/src/response.rs#L42), so the user can click through to it.";

/// The tool as OpenCode's config takes it, under `mcp`, started with
/// `command`.
pub fn opencode_server(command: &Path) -> Value {
    json!({
        "type": "local",
        "command": [command, FLAG],
        "enabled": true,
    })
}

/// The `mcpServers` of an ACP conversation in `directory`, the tool
/// started with `command`; none without one. ACP has no working folder
/// for a server, so it's told the project.
pub fn acp_servers(command: Option<&Path>, directory: &str) -> Value {
    match command {
        Some(command) => json!([{
            "name": NAME,
            "command": command,
            "args": [FLAG, "--project", directory],
            "env": [],
        }]),
        None => json!([]),
    }
}

/// The arguments after [`FLAG`]: `--project <folder>`, else the folder it
/// was started in.
pub fn project_from_args(args: &[String]) -> Result<PathBuf> {
    match args.iter().position(|arg| arg == "--project") {
        Some(at) => args
            .get(at + 1)
            .map(PathBuf::from)
            .context("--project needs a folder"),
        None => std::env::current_dir().context("No working folder"),
    }
}

/// Answer requests from `input` on `output` until the agent hangs up.
pub fn serve(store: &impl TaskStore, input: impl BufRead, mut output: impl Write) -> Result<()> {
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
        // Notifications get no answer.
        let Some(id) = message.get("id").cloned() else {
            continue;
        };
        let method = message["method"].as_str().unwrap_or_default();
        let answer = match handle(store, method, &message["params"]) {
            Ok(result) => json!({"jsonrpc": "2.0", "id": id, "result": result}),
            Err(code) => {
                json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": format!("Unknown method {method}")}})
            }
        };
        writeln!(output, "{answer}")?;
        output.flush()?;
    }
    Ok(())
}

fn handle(store: &impl TaskStore, method: &str, params: &Value) -> Result<Value, i64> {
    Ok(match method {
        "initialize" => json!({
            "protocolVersion": params["protocolVersion"].as_str().unwrap_or("2025-06-18"),
            "capabilities": {"tools": {}},
            "serverInfo": {"name": NAME, "title": "Jig tasks", "version": env!("CARGO_PKG_VERSION")},
            "instructions": format!("{INSTRUCTIONS} {SHAPE} {LINKS}"),
        }),
        "ping" => json!({}),
        "tools/list" => json!({"tools": tools()}),
        "tools/call" => {
            let name = params["name"].as_str().unwrap_or_default();
            let arguments = &params["arguments"];
            match call(store, name, arguments) {
                Ok(text) => json!({"content": [{"type": "text", "text": text}]}),
                Err(error) => json!({
                    "content": [{"type": "text", "text": format!("{error:#}")}],
                    "isError": true,
                }),
            }
        }
        _ => return Err(-32601),
    })
}

fn tools() -> Value {
    let link = LINKS;
    json!([
        {
            "name": "create_task",
            "title": "Create a task",
            "description": format!("Save a plan as a task in Jig's task planner, which the user sees in the sidebar and works through. Call this whenever you propose three or more steps, or work across several files, rather than asking in your reply or with a question tool: Jig shows the user the plan and asks whether to save it. If they turn it down, the call fails, maybe with what to change; follow that. {SHAPE} {link}"),
            "inputSchema": {
                "type": "object",
                "properties": {
                    "title": {"type": "string", "description": "Short, like a commit subject."},
                    "description": {"type": "string", "description": format!("Markdown: a one or two sentence summary, then `### Background`, `### Context`, `### Purpose` and `### Risks`, each a sentence or two, and nothing else. {link}")},
                    "todos": {"type": "array", "items": {"type": "string"}, "description": format!("The steps, in order, each one short line of Markdown. {link}")},
                },
                "required": ["title", "todos"],
            },
        },
        {
            "name": "list_tasks",
            "title": "List tasks",
            "description": "The project's tasks in Jig's task planner, newest first, with their ids and numbered todos.",
            "inputSchema": {"type": "object", "properties": {}},
            "annotations": {"readOnlyHint": true},
        },
        {
            "name": "update_task",
            "title": "Update a task",
            "description": format!("Change a task in Jig's task planner: check off or uncheck todos by their number from list_tasks, add todos, or change the title or description. {link}"),
            "inputSchema": {
                "type": "object",
                "properties": {
                    "id": {"type": "string", "description": "The task's id, from create_task or list_tasks."},
                    "check": {"type": "array", "items": {"type": "integer"}, "description": "Numbers of the todos now done, from 1."},
                    "uncheck": {"type": "array", "items": {"type": "integer"}, "description": "Numbers of the todos not done after all, from 1."},
                    "add_todos": {"type": "array", "items": {"type": "string"}, "description": "Todos to add at the end."},
                    "title": {"type": "string"},
                    "description": {"type": "string", "description": "Replaces the whole description."},
                },
                "required": ["id"],
            },
        },
    ])
}

fn call(store: &impl TaskStore, name: &str, arguments: &Value) -> Result<String> {
    let text = |name: &str| arguments[name].as_str().map(str::to_string);
    let strings = |name: &str| -> Vec<String> {
        arguments[name]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|item| item.as_str().map(str::to_string))
            .filter(|item| !item.trim().is_empty())
            .collect()
    };
    let numbers = |name: &str| -> Vec<usize> {
        arguments[name]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_u64)
            .map(|n| n as usize)
            .collect()
    };
    match name {
        "create_task" => {
            let todos = strings("todos")
                .iter()
                .map(|todo| Todo::new(todo))
                .collect();
            let task = super::create(
                store,
                &text("title").unwrap_or_default(),
                &text("description").unwrap_or_default(),
                todos,
            )?;
            Ok(format!(
                "Saved the task \"{}\" (id {}) with {} todos. The user sees it in Jig's task planner.",
                task.title,
                task.id,
                task.todos.len()
            ))
        }
        "list_tasks" => {
            let tasks = store.list()?;
            if tasks.is_empty() {
                return Ok("There are no tasks yet.".into());
            }
            Ok(tasks
                .iter()
                .map(|task| {
                    let todos: Vec<String> = task
                        .todos
                        .iter()
                        .enumerate()
                        .map(|(ix, todo)| {
                            let check = if todo.done { 'x' } else { ' ' };
                            format!("  {}. [{check}] {}", ix + 1, todo.text)
                        })
                        .collect();
                    format!("{} (id {})\n{}", task.title, task.id, todos.join("\n"))
                })
                .collect::<Vec<_>>()
                .join("\n\n"))
        }
        "update_task" => {
            let id = text("id").unwrap_or_default();
            let mut task = store.load(&id)?;
            for (field, done) in [("check", true), ("uncheck", false)] {
                for n in numbers(field) {
                    let todo = n
                        .checked_sub(1)
                        .and_then(|ix| task.todos.get_mut(ix))
                        .with_context(|| format!("The task has no todo {n}."))?;
                    todo.done = done;
                }
            }
            task.todos
                .extend(strings("add_todos").iter().map(|todo| Todo::new(todo)));
            if let Some(title) = text("title").filter(|title| !title.trim().is_empty()) {
                task.title = title;
            }
            if let Some(description) = text("description") {
                task.description = description;
            }
            store.save(&task)?;
            Ok(format!(
                "Updated \"{}\": {} of {} todos done.",
                task.title,
                task.done(),
                task.todos.len()
            ))
        }
        other => anyhow::bail!("There's no tool {other}."),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Send `requests`, one per line, and read back the answers.
    fn exchange(project: &Path, requests: &[Value]) -> Vec<Value> {
        let input: String = requests.iter().map(|r| format!("{r}\n")).collect();
        let mut output = Vec::new();
        crate::store::persist::tasks::serve(project, input.as_bytes(), &mut output).unwrap();
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
    fn starts_like_an_mcp_server() {
        let project = tempfile::tempdir().unwrap();
        let answers = exchange(
            project.path(),
            &[
                json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-03-26", "capabilities": {}}}),
                json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
                json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}),
                json!({"jsonrpc": "2.0", "id": 3, "method": "resources/list"}),
            ],
        );
        assert_eq!(answers.len(), 3, "the notification gets no answer");
        assert_eq!(answers[0]["result"]["protocolVersion"], "2025-03-26");
        assert!(
            answers[0]["result"]["instructions"]
                .as_str()
                .unwrap()
                .contains("call create_task with the plan")
        );
        let create = &answers[1]["result"]["tools"][0]["description"];
        assert!(
            create.as_str().unwrap().contains("`### Risks`"),
            "told the shape"
        );
        let names: Vec<&str> = answers[1]["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|tool| tool["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, TOOLS);
        assert_eq!(answers[2]["error"]["code"], -32601);
    }

    #[test]
    fn creates_lists_and_checks_off_tasks() {
        let project = tempfile::tempdir().unwrap();
        let answers = exchange(
            project.path(),
            &[
                call_tool(
                    1,
                    "create_task",
                    json!({"title": "Add a planner", "description": "See [`list`](src/tasks.rs#L80).", "todos": ["Store", "Show"]}),
                ),
                call_tool(
                    2,
                    "update_task",
                    json!({"id": "add-a-planner", "check": [1], "add_todos": ["Link"]}),
                ),
                call_tool(3, "list_tasks", json!({})),
                call_tool(
                    4,
                    "update_task",
                    json!({"id": "add-a-planner", "check": [9]}),
                ),
                call_tool(5, "update_task", json!({"id": "../../etc/passwd"})),
            ],
        );
        let text = |ix: usize| {
            answers[ix]["result"]["content"][0]["text"]
                .as_str()
                .unwrap()
                .to_string()
        };
        assert!(text(0).contains("id add-a-planner"), "{}", text(0));
        assert!(text(1).contains("1 of 3"), "{}", text(1));
        assert!(text(2).contains("1. [x] Store"), "{}", text(2));
        assert!(text(2).contains("3. [ ] Link"), "{}", text(2));
        assert_eq!(answers[3]["result"]["isError"], true);
        assert_eq!(answers[4]["result"]["isError"], true);
        let task = crate::store::persist::tasks::load(project.path(), "add-a-planner").unwrap();
        assert_eq!(task.description, "See [`list`](src/tasks.rs#L80).");
        assert_eq!(task.done(), 1);
    }

    #[test]
    fn takes_the_project_from_its_arguments() {
        let args = ["--project".to_string(), "/p".to_string()];
        assert_eq!(project_from_args(&args).unwrap(), PathBuf::from("/p"));
        assert!(project_from_args(&args[..1]).is_err());
    }
}
