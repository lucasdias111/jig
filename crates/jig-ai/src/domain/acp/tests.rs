use super::*;
use crate::domain::agent::{AgentEvent, EditChange};

#[test]
fn finds_the_model_setting_among_the_options() {
    let options = json!([
        {"id": "mode", "category": "mode", "type": "select", "currentValue": "default",
         "options": [{"value": "default", "name": "Manual"}]},
        {"id": "model", "category": "model", "type": "select", "currentValue": "default",
         "options": [{"value": "default", "name": "Default (recommended)"},
                     {"value": "opus", "name": "Opus 5.5"}]},
    ]);
    let models = AgentModels::from_options(&options).unwrap();
    assert_eq!(models.config_id, "model");
    assert_eq!(models.current, "default");
    assert_eq!(models.options[1], ("opus".into(), "Opus 5.5".into()));

    let grouped = json!([{"id": "m", "category": "model", "type": "select", "currentValue": "a",
            "options": [{"group": "x", "name": "X", "options": [{"value": "a", "name": "A"}]}]}]);
    assert_eq!(
        AgentModels::from_options(&grouped).unwrap().options,
        [("a".into(), "A".into())]
    );
    assert_eq!(AgentModels::from_options(&json!([])), None);
}

#[test]
fn parses_iso_times() {
    let at = parse_time("2026-10-10T05:38:50.337Z").unwrap();
    let seconds = at.duration_since(SystemTime::UNIX_EPOCH).unwrap().as_secs();
    assert_eq!(seconds, 1_791_610_730);
    assert_eq!(
        parse_time("1970-01-02T00:00:00Z"),
        Some(SystemTime::UNIX_EPOCH + Duration::from_secs(86_400))
    );
    assert_eq!(parse_time("yesterday"), None);
}

#[test]
fn an_edit_comes_from_its_diff_or_its_input() {
    let mut call = ToolCall::default();
    call.merge(&json!({
        "toolCallId": "c1", "kind": "edit", "title": "edit",
        "content": [{"type": "diff", "path": "/p/a.rs", "oldText": "a", "newText": "b"}],
    }));
    let edit = call.edit().unwrap();
    assert_eq!(edit.path, PathBuf::from("/p/a.rs"));
    assert_eq!(
        edit.change,
        EditChange::Replace {
            old: Some("a".into()),
            new: "b".into()
        }
    );

    let mut call = ToolCall::default();
    call.merge(&json!({
        "toolCallId": "c2", "kind": "edit",
        "rawInput": {"file_path": "/p/b.rs", "old_string": "x", "new_string": "y"},
    }));
    assert_eq!(call.edit().unwrap().path, PathBuf::from("/p/b.rs"));

    let mut run = ToolCall::default();
    run.merge(
        &json!({"toolCallId": "c3", "kind": "execute", "rawInput": {"command": ["cargo", "test"]}}),
    );
    assert!(run.edit().is_none());
    assert_eq!(run.detail("/p"), "cargo test");
    assert_eq!(run.asking(), "Run a command");
}

#[test]
fn a_turn_collects_the_reply_and_flags_edits_made_without_asking() {
    let mut turn = TurnState::new("/p", Permissions::default());
    let chunk = |id: &str, text: &str| json!({"sessionUpdate": "agent_message_chunk", "messageId": id, "content": {"type": "text", "text": text}});
    turn.update(&chunk("m1", "Looking"));
    turn.update(&chunk("m1", " now."));
    turn.update(&chunk("m2", "Done."));
    assert_eq!(turn.reply(), "Looking now.\n\nDone.");

    let events = turn.update(&json!({
            "sessionUpdate": "tool_call_update", "toolCallId": "e1", "kind": "edit", "status": "completed",
            "content": [{"type": "diff", "path": "/p/src/a.rs", "oldText": "a", "newText": "b"}],
        }));
    assert_eq!(
        events,
        [AgentEvent::Did("Edited src/a.rs without asking".into())]
    );

    turn.reviewed("e2");
    let events = turn.update(&json!({
            "sessionUpdate": "tool_call_update", "toolCallId": "e2", "kind": "edit", "status": "completed",
            "content": [{"type": "diff", "path": "/p/src/b.rs", "newText": "b"}],
        }));
    assert!(events.is_empty(), "it was reviewed");
}

#[test]
fn a_drawing_on_the_map_is_reported_once_with_what_it_was_given() {
    let mut turn = TurnState::new("/p", Permissions::default());
    let started = turn.update(&json!({
        "sessionUpdate": "tool_call", "toolCallId": "d1", "status": "pending",
        "title": "mcp__jig-map__map_trace", "kind": "other",
        "rawInput": {"title": "Booking", "steps": [{"node": "a.rs#book", "label": "Book"}]},
    }));
    assert!(matches!(started.as_slice(), [AgentEvent::Step(_)]));
    let update =
        json!({"sessionUpdate": "tool_call_update", "toolCallId": "d1", "status": "completed"});
    assert_eq!(
        turn.update(&update),
        [AgentEvent::Drew {
            tool: "map_trace".into(),
            input: json!({"title": "Booking", "steps": [{"node": "a.rs#book", "label": "Book"}]}),
        }]
    );
    assert!(turn.update(&update).is_empty(), "only once");
}

#[test]
fn a_subagents_work_and_report_are_its_own() {
    let mut turn = TurnState::new("/p", Permissions::default());
    let spawned = turn.update(&json!({
        "sessionUpdate": "subagent_spawned", "subagentSessionId": "s1",
        "name": "Find the loader", "task": "Look for where config is read", "capabilities": {},
    }));
    assert_eq!(
        spawned,
        [AgentEvent::Subagent {
            id: "s1".into(),
            description: "Find the loader".into()
        }]
    );

    // Its reads and searches are listed under it, not the conversation.
    let read = turn.subagent_update(
        "s1",
        &json!({
            "sessionUpdate": "tool_call", "toolCallId": "r1", "kind": "read", "status": "completed",
            "title": "Read", "locations": [{"path": "/p/src/config.rs"}],
        }),
    );
    assert_eq!(
        read,
        [AgentEvent::SubagentDid {
            id: "s1".into(),
            text: "Read src/config.rs".into()
        }]
    );
    let ran = turn.subagent_update(
            "s1",
            &json!({
                "sessionUpdate": "tool_call", "toolCallId": "x1", "kind": "execute", "status": "completed",
                "rawInput": {"command": "rg load"},
            }),
        );
    assert_eq!(
        ran,
        [AgentEvent::SubagentDid {
            id: "s1".into(),
            text: "Ran `rg load`".into()
        }]
    );

    // Its words are its report, not the conversation's reply.
    let chunk = json!({"sessionUpdate": "agent_message_chunk", "messageId": "m1", "content": {"type": "text", "text": "It's in config.rs."}});
    turn.subagent_update("s1", &chunk);
    assert_eq!(turn.reply(), "");
    let done = turn.update(&json!({
        "sessionUpdate": "subagent_state_update", "subagentSessionId": "s1", "state": "completed",
    }));
    assert_eq!(
        done,
        [AgentEvent::SubagentDone {
            id: "s1".into(),
            report: "It's in config.rs.".into()
        }]
    );

    // An edit it makes without asking still shows in the conversation.
    let edited = turn.subagent_update(
            "s1",
            &json!({
                "sessionUpdate": "tool_call_update", "toolCallId": "e1", "kind": "edit", "status": "completed",
                "content": [{"type": "diff", "path": "/p/src/a.rs", "newText": "b"}],
            }),
        );
    assert_eq!(
        edited,
        [AgentEvent::Did("Edited src/a.rs without asking".into())]
    );
}

#[test]
fn jigs_task_tools_are_known_by_name() {
    let call = |title: &str| ToolCall {
        title: title.into(),
        kind: "other".into(),
        ..ToolCall::default()
    };
    assert!(call("mcp__jig__create_task").is_task_tool());
    assert!(call("update_task (jig MCP Server)").is_task_tool());
    assert!(!call("mcp__other__create_task").is_task_tool());
    assert!(!call("mcp__jig__something_else").is_task_tool());
    assert!(call("mcp__jig__create_task").creates_task());
    assert!(!call("mcp__jig__update_task").creates_task());
    assert_eq!(Permissions::default().tasks, Access::Allow);
}

#[test]
fn settings_decide_by_the_kind_of_tool() {
    let permissions = Permissions {
        shell: Access::Deny,
        web_fetch: Access::Allow,
        ..Permissions::default()
    };
    assert_eq!(permissions.access("execute"), Access::Deny);
    assert_eq!(permissions.access("fetch"), Access::Allow);
    assert_eq!(permissions.access("read"), Access::Allow);
    assert_eq!(permissions.access("delete"), Access::Ask);
}

#[test]
fn a_replayed_conversation_becomes_the_transcript() {
    let mut transcript = Transcript::new("/p");
    for update in [
        json!({"sessionUpdate": "user_message_chunk", "content": {"type": "text", "text": "Add docs\n\nI'm in a.rs, with line 1 selected"}}),
        json!({"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": "On it"}}),
        json!({"sessionUpdate": "tool_call", "toolCallId": "t", "kind": "edit", "status": "completed",
                   "content": [{"type": "diff", "path": "/p/a.rs", "newText": "x"}]}),
        json!({"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": "Done."}}),
    ] {
        transcript.add(&update);
    }
    use crate::domain::agent::Turn as Line;
    assert_eq!(
        transcript.finish(),
        [
            Line::User("Add docs".into()),
            Line::Agent("On it".into()),
            Line::Edit {
                path: "a.rs".into(),
                accepted: true
            },
            Line::Agent("Done.".into()),
        ]
    );
}
