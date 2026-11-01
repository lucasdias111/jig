use super::events::*;
use super::transcript::*;
use super::*;
use crate::domain::agent::*;
use crate::domain::config::AgentModel;

#[test]
fn subagents_are_followed_by_their_sessions() {
    let mut sessions = Sessions::new("ses_own");
    assert!(sessions.adopt("ses_a", "ses_own"), "a subagent");
    assert!(!sessions.adopt("ses_a", "ses_own"), "known already");
    assert!(sessions.adopt("ses_b", "ses_a"), "a subagent's subagent");
    assert!(
        !sessions.adopt("ses_c", "ses_other"),
        "another conversation's"
    );
    assert!(sessions.follows("ses_own") && sessions.follows("ses_b"));
    assert!(!sessions.follows("ses_c"));
}

#[test]
fn a_subagent_is_shown_by_its_task_and_report() {
    assert_eq!(
        subagent_title("Find the loader (@explore subagent)"),
        "Find the loader"
    );
    assert_eq!(subagent_title(""), "Subagent");
    let state = json!({
        "status": "completed",
        "output": "It's in config.rs.\n\n<task_metadata>\nsession_id: ses_a\n</task_metadata>",
    });
    assert_eq!(task_report(&state), "It's in config.rs.");
    assert_eq!(
        subagent_did("read", &json!({"filePath": "/p/src/a.rs"}), "/p"),
        "Read src/a.rs"
    );
    assert_eq!(
        subagent_did("bash", &json!({"command": "cargo test"}), "/p"),
        "Ran `cargo test`"
    );
}

#[test]
fn describes_tool_calls_relative_to_the_project() {
    let input = json!({"filePath": "/p/src/lib.rs"});
    assert_eq!(describe("read", &input, "/p"), "Reading src/lib.rs");
    assert_eq!(
        describe("grep", &json!({"pattern": "fn main"}), "/p"),
        "Searching for fn main"
    );
}

#[test]
fn lists_only_the_conversations_jig_started() {
    let sessions = json!([
        {"id": "ses_old", "title": "Jig", "time": {"updated": 1000},
         "permission": [{"permission": "edit", "pattern": "*", "action": "ask"}]},
        {"id": "ses_new", "title": "Add docs", "metadata": {"jig": true}, "time": {"updated": 2000}},
        {"id": "ses_tui", "title": "Mine", "time": {"updated": 3000}, "permission": []},
        {"id": "ses_child", "parentID": "ses_new", "metadata": {"jig": true}, "time": {"updated": 4000}},
        {"id": "ses_gone", "metadata": {"jig": true}, "time": {"updated": 5000, "archived": 5000}},
    ]);
    let found = conversations(&sessions);
    let ids: Vec<_> = found.iter().map(|c| c.id.as_str()).collect();
    assert_eq!(ids, ["ses_new", "ses_old"], "latest first");
    assert_eq!(found[0].title.as_deref(), Some("Add docs"));
    assert_eq!(found[1].title, None, "untitled from before");
}

#[test]
fn history_keeps_what_was_said_and_done() {
    let messages = json!([
        {"info": {"role": "user"}, "parts": [
            {"type": "text", "text": "Add docs\n\nI'm in src/lib.rs, with line 2 selected:\n```rust\nfn b() {}\n```"},
        ]},
        {"info": {"role": "assistant"}, "parts": [
            {"type": "step-start"},
            {"type": "tool", "tool": "read", "state": {"status": "completed", "input": {"filePath": "/p/src/lib.rs"}}},
            {"type": "tool", "tool": "bash", "state": {"status": "completed", "input": {"command": "cargo test"}}},
            {"type": "tool", "tool": "edit", "state": {"status": "completed", "input": {"filePath": "/p/src/lib.rs"}}},
            {"type": "tool", "tool": "edit", "state": {"status": "error", "input": {"filePath": "/p/src/main.rs"}}},
            {"type": "tool", "tool": "jig-map_map_highlight", "state": {"status": "completed", "input": {"title": "Docs", "nodes": ["src/lib.rs"]}}},
            {"type": "text", "text": "Documented `b`."},
        ]},
    ]);
    assert_eq!(
        history(&messages, "/p"),
        [
            Turn::User("Add docs".into()),
            Turn::Did("Ran `cargo test`".into()),
            Turn::Edit {
                path: "src/lib.rs".into(),
                accepted: true
            },
            Turn::Edit {
                path: "src/main.rs".into(),
                accepted: false
            },
            Turn::Drew {
                tool: "map_highlight".into(),
                input: json!({"title": "Docs", "nodes": ["src/lib.rs"]}),
            },
            Turn::Agent("Documented `b`.".into()),
        ]
    );
}

#[test]
fn asks_about_commands_and_the_web_but_not_the_rest() {
    let bash =
        json!({"id": "per_1", "permission": "bash", "patterns": ["cargo test"], "metadata": {}});
    assert_eq!(
        permission(&bash, None),
        Some(PermissionRequest {
            id: "per_1".into(),
            title: "Run a command".into(),
            detail: "cargo test".into(),
            kind: Asking::Command,
            always: Some("Always allow".into()),
            task: None,
        })
    );
    let with_pattern = json!({"id": "per_4", "permission": "bash", "patterns": ["cargo test"], "always": ["cargo test *"], "metadata": {}});
    assert_eq!(
        permission(&with_pattern, None).unwrap().always.as_deref(),
        Some("Always allow `cargo test *`"),
        "says what OpenCode will remember"
    );
    let fetch = json!({"id": "per_2", "permission": "webfetch", "patterns": [], "metadata": {"url": "https://x.dev"}});
    assert_eq!(permission(&fetch, None).unwrap().detail, "https://x.dev");
    let other = json!({"id": "per_3", "permission": "external_directory", "patterns": ["/etc"], "metadata": {}});
    assert_eq!(permission(&other, None), None);
}

#[test]
fn saving_a_task_asks_about_the_plan_it_was_called_with() {
    let mut sessions = Sessions::new("ses_1");
    sessions.keep_draft(&json!({
            "type": "tool", "tool": "jig_create_task", "callID": "call_1",
            "state": {"status": "running", "input": {"title": "Split the parser", "todos": ["Move the lexer", " ", "Add tests"]}},
        }));
    let asked = json!({
        "id": "per_1", "permission": "jig_create_task", "patterns": ["*"], "always": ["*"],
        "metadata": {}, "tool": {"messageID": "msg_1", "callID": "call_1"},
    });
    let draft = sessions.draft_for(&asked);
    assert!(draft.is_some());
    let request = permission(&asked, draft).unwrap();
    assert_eq!(request.title, "Save this plan as a task");
    assert_eq!(
        request.task,
        Some(TaskDraft {
            title: "Split the parser".into(),
            todos: vec!["Move the lexer".into(), "Add tests".into()],
        })
    );
}

#[test]
fn permissions_become_opencode_rules() {
    let permissions = Permissions {
        shell: Access::Allow,
        web_search: Access::Deny,
        questions: false,
        ..Permissions::default()
    };
    let set = rules(&permissions);
    let action = |name: &str| {
        set.as_array()
            .unwrap()
            .iter()
            .find(|rule| rule["permission"] == name)
            .map(|rule| rule["action"].as_str().unwrap().to_string())
    };
    assert_eq!(action("bash").as_deref(), Some("allow"));
    assert_eq!(action("websearch").as_deref(), Some("deny"));
    assert_eq!(action("webfetch").as_deref(), Some("ask"));
    assert_eq!(action("question").as_deref(), Some("deny"));
    assert_eq!(action("edit").as_deref(), Some("ask"), "always");
    assert_eq!(
        action("jig_update_task").as_deref(),
        Some("allow"),
        "by default"
    );
    assert_eq!(
        action("jig_create_task").as_deref(),
        Some("ask"),
        "the user is asked whether to save the plan"
    );
    let off = rules(&Permissions {
        tasks: Access::Deny,
        ..Permissions::default()
    });
    assert!(
        off.as_array()
            .unwrap()
            .contains(&json!({"permission": "jig_create_task", "pattern": "*", "action": "deny"}))
    );
    let asking = rules(&Permissions {
        tasks: Access::Ask,
        ..Permissions::default()
    });
    assert!(
        asking
            .as_array()
            .unwrap()
            .contains(&json!({"permission": "jig_update_task", "pattern": "*", "action": "ask"}))
    );
    assert_eq!(
        action("external_directory").as_deref(),
        Some("deny"),
        "always"
    );
    assert_eq!(
        action("jig-map_map_trace").as_deref(),
        Some("allow"),
        "drawing on the Map changes no code"
    );
}

#[test]
fn base64_matches_known_values() {
    assert_eq!(base64(b"opencode:pw"), "b3BlbmNvZGU6cHc=");
    assert_eq!(base64(b"ab"), "YWI=");
}

#[test]
fn jig_providers_are_handed_to_opencode_under_their_own_name() {
    let config = crate::Config::parse(
            "[[provider]]\nname = \"openrouter\"\nkind = \"openai\"\nbase_url = \"https://openrouter.ai/api/v1/\"\n",
        )
        .unwrap();
    let agent = AgentModel::Jig {
        provider: config.providers[0].clone(),
        model: "anthropic/claude-sonnet-5-5".into(),
    };
    let target = Target::new(&agent, Some("sk"));
    assert_eq!(target.model, "jig-openrouter/anthropic/claude-sonnet-5-5");
    let provider = &target.config.unwrap()["provider"]["jig-openrouter"];
    assert_eq!(provider["npm"], "@ai-sdk/openai-compatible");
    assert_eq!(
        provider["options"]["baseURL"],
        "https://openrouter.ai/api/v1"
    );
    assert_eq!(provider["options"]["apiKey"], "sk");
    assert_eq!(
        provider["models"]["anthropic/claude-sonnet-5-5"]["tool_call"],
        true
    );

    let native = Target::new(&AgentModel::OpenCode(DEFAULT_MODEL.into()), None);
    assert_eq!(
        native,
        Target {
            model: DEFAULT_MODEL.into(),
            config: None
        }
    );
}
