use std::io::{BufReader, Write};

use futures::StreamExt as _;
use serde_json::json;

use super::*;

#[test]
fn requests_go_out_framed_and_replies_come_back_in_order() {
    let (ours, theirs) = socket_pair();
    let (tx, rx) = futures::channel::mpsc::unbounded();
    let client = Client::connect(7, ours.try_clone().unwrap(), ours, tx);

    let seq = client.request("initialize", json!({"adapterID": "lldb-dap"}));
    let mut adapter = BufReader::new(theirs.try_clone().unwrap());
    let request = read_message(&mut adapter).unwrap();
    assert_eq!(request["command"], "initialize");
    assert_eq!(request["seq"], seq);

    let mut out = theirs;
    for message in [
        json!({"seq": 1, "type": "event", "event": "initialized"}),
        json!({"seq": 2, "type": "response", "request_seq": seq, "success": true,
                   "command": "initialize", "body": {"supportsStepBack": false}}),
        json!({"seq": 3, "type": "request", "command": "runInTerminal", "arguments": {}}),
    ] {
        out.write_all(&frame(&message)).unwrap();
    }
    // The reverse request is declined.
    let reply = read_message(&mut adapter).unwrap();
    assert_eq!(reply["request_seq"], 3);
    assert_eq!(reply["success"], false);
    drop(out);
    drop(adapter);
    drop(client);

    let messages: Vec<Message> = futures::executor::block_on(rx.collect::<Vec<_>>())
        .into_iter()
        .map(|(id, message)| {
            assert_eq!(id, 7);
            message
        })
        .collect();
    assert_eq!(
        messages,
        [
            Message::Event {
                event: "initialized".into(),
                body: Value::Null
            },
            Message::Response {
                request_seq: seq,
                success: true,
                body: json!({"supportsStepBack": false}),
                message: None,
            },
            Message::Closed,
        ]
    );
}

/// Talks to the real `lldb-dap` about a tiny Rust program: the order of
/// messages the debug session relies on. Needs Xcode's tools and rustc.
#[test]
#[ignore]
fn lldb_dap_live() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("main.rs");
    std::fs::write(
            &source,
            "fn main() {\n    let numbers = vec![1, 2, 3];\n    let total: i32 = numbers.iter().sum();\n    println!(\"total {total}\");\n}\n",
        )
        .unwrap();
    let program = dir.path().join("main");
    let built = std::process::Command::new("rustc")
        .args(["-g", "-o"])
        .arg(&program)
        .arg(&source)
        .status()
        .unwrap();
    assert!(built.success());

    let adapter = AdapterCommand {
        program: jig_debuggers::lldb_dap().expect("lldb-dap"),
        args: Vec::new(),
        env: Vec::new(),
        server: false,
        shown: PathBuf::new(),
    };
    let (tx, rx) = futures::channel::mpsc::unbounded();
    let client = Client::start(0, &adapter, dir.path(), tx).unwrap();
    let mut rx = futures::executor::block_on_stream(rx).map(|(_, message)| message);
    let mut log = Vec::new();
    let mut next = |log: &mut Vec<String>| {
        let message = rx.next().expect("adapter hung up");
        log.push(format!("{message:?}").chars().take(160).collect());
        message
    };
    let initialize = client.request("initialize", json!({"adapterID": "lldb-dap", "linesStartAt1": true, "columnsStartAt1": true, "pathFormat": "path"}));
    loop {
        if let Message::Response {
            request_seq,
            success,
            ..
        } = next(&mut log)
            && request_seq == initialize
        {
            assert!(success);
            break;
        }
    }
    let config = jig_run::run_configs::RunConfig {
        name: "t".into(),
        command: String::new(),
        cwd: dir.path().to_path_buf(),
        env: Vec::new(),
        source: jig_run::run_configs::Source::File,
        debug: Some(jig_run::run_configs::DebugProgram {
            program: program.clone(),
            args: Vec::new(),
            build: None,
        }),
        debugger: None,
    };
    let arguments = crate::debug_target::plan_for(&config, &jig_debuggers::registry())
        .unwrap()
        .launch_arguments(&config, dir.path(), None, &[])
        .unwrap();
    let launch = client.request("launch", arguments);
    let mut stopped_thread = None;
    while stopped_thread.is_none() {
        match next(&mut log) {
            Message::Event { event, .. } if event == "initialized" => {
                client.request(
                    "setBreakpoints",
                    json!({"source": {"path": source}, "breakpoints": [{"line": 4}]}),
                );
                client.request("configurationDone", json!({}));
            }
            Message::Event { event, body } if event == "stopped" => {
                stopped_thread = body["threadId"].as_i64();
            }
            Message::Response {
                request_seq,
                success,
                message,
                ..
            } if request_seq == launch => {
                assert!(success, "launch failed: {message:?} {log:#?}");
            }
            Message::Closed => panic!("closed early: {log:#?}"),
            _ => {}
        }
    }
    let trace = client.request(
        "stackTrace",
        json!({"threadId": stopped_thread.unwrap(), "levels": 20}),
    );
    let frame = loop {
        if let Message::Response {
            request_seq, body, ..
        } = next(&mut log)
            && request_seq == trace
        {
            let top = &body["stackFrames"][0];
            assert_eq!(top["line"], 4, "{body}");
            assert!(top["source"]["path"].as_str().unwrap().ends_with("main.rs"));
            break top["id"].as_i64().unwrap();
        }
    };
    let scopes = client.request("scopes", json!({"frameId": frame}));
    let locals = loop {
        if let Message::Response {
            request_seq, body, ..
        } = next(&mut log)
            && request_seq == scopes
        {
            break body["scopes"][0]["variablesReference"].as_i64().unwrap();
        }
    };
    let variables = client.request("variables", json!({"variablesReference": locals}));
    loop {
        if let Message::Response {
            request_seq, body, ..
        } = next(&mut log)
            && request_seq == variables
        {
            let names: Vec<&str> = body["variables"]
                .as_array()
                .unwrap()
                .iter()
                .filter_map(|v| v["name"].as_str())
                .collect();
            assert!(
                names.contains(&"total") && names.contains(&"numbers"),
                "{body}"
            );
            eprintln!("{body:#}");
            break;
        }
    }
    client.request("disconnect", json!({"terminateDebuggee": true}));
    eprintln!("{log:#?}");
}
