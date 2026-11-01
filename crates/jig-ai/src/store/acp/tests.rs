use std::sync::atomic::Ordering;

use super::*;
use crate::domain::agent::{AgentRequest, Permissions};

/// An agent that wants a sign-in: it answers `initialize` with a
/// terminal login, and every `session/new` with "Authentication required".
#[cfg(unix)]
fn agent_wanting_a_sign_in() -> AgentCommandLine {
    let script = r#"
read -r line
printf '%s\n' '{"jsonrpc":"2.0","id":0,"result":{"protocolVersion":1,"agentCapabilities":{},"authMethods":[{"id":"login","name":"Log in","description":"Use your account ","type":"terminal","args":["--cli","login"]},{"id":"browser","name":"Browser"}]}}'
read -r line
printf '%s\n' '{"jsonrpc":"2.0","id":1,"error":{"code":-32000,"message":"Authentication required"}}'
read -r line
"#;
    AgentCommandLine {
        program: "/bin/sh".into(),
        args: vec!["-c".into(), script.into(), "agent".into()],
        env: Vec::new(),
        path: None,
    }
}

#[cfg(unix)]
#[test]
fn a_second_turn_is_refused_while_the_first_runs() {
    // Answers `initialize` and `session/new`, then never finishes a turn.
    let script = r#"
read -r line
printf '%s\n' '{"jsonrpc":"2.0","id":0,"result":{"protocolVersion":1,"agentCapabilities":{}}}'
read -r line
printf '%s\n' '{"jsonrpc":"2.0","id":1,"result":{"sessionId":"s"}}'
read -r line
sleep 30
"#;
    let command = AgentCommandLine {
        program: "/bin/sh".into(),
        args: vec!["-c".into(), script.into(), "agent".into()],
        env: Vec::new(),
        path: None,
    };
    let connection = Connection::start("Fake", &command).unwrap();
    let session = Session::create(connection.clone(), Path::new("/"), None).unwrap();
    let request = AgentRequest {
        directory: "/".into(),
        prompt: "Hi".into(),
        model: String::new(),
        command: None,
        permissions: Permissions::default(),
    };
    let first = {
        let (session, request) = (session.clone(), request.clone());
        std::thread::spawn(move || session.run(&request, &|_| {}))
    };
    while !session.busy.load(Ordering::SeqCst) {
        std::thread::sleep(Duration::from_millis(5));
    }
    let error = session.run(&request, &|_| {}).unwrap_err();
    assert_eq!(
        error.to_string(),
        "Fake is still working on this conversation."
    );
    connection.stop();
    assert!(first.join().unwrap().is_err(), "the agent stopped");
    assert!(!session.busy.load(Ordering::SeqCst), "free once it ends");
}

#[cfg(unix)]
#[test]
fn says_when_the_agent_wants_a_sign_in_and_how() {
    let command = agent_wanting_a_sign_in();
    let connection = Connection::start("Fake", &command).unwrap();
    let error = match Session::create(connection, Path::new("/"), None) {
        Err(error) => error,
        Ok(_) => panic!("it wants a sign-in first"),
    };
    let needed = error.downcast_ref::<SignInNeeded>().expect("a sign-in");
    assert_eq!(needed.to_string(), "Fake needs you to sign in.");
    let [terminal, browser] = needed.methods.as_slice() else {
        panic!("two ways: {:?}", needed.methods);
    };
    assert_eq!(terminal.name, "Log in");
    assert_eq!(terminal.description, "Use your account");
    let login = terminal.terminal.as_ref().expect("in a terminal");
    assert_eq!(login.program, command.program);
    assert_eq!(
        login.args[3..],
        ["--cli", "login"],
        "after its own arguments"
    );
    assert_eq!(browser.terminal, None, "the agent does it itself");
}

#[test]
fn reads_the_lines_asked_for() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("f.txt");
    std::fs::write(&path, "one\ntwo\nthree\n").unwrap();
    let read = |line: Option<u64>, limit: Option<u64>| {
        read_file(&json!({"path": path, "line": line, "limit": limit})).unwrap()["content"]
            .as_str()
            .unwrap()
            .to_string()
    };
    assert_eq!(read(None, None), "one\ntwo\nthree\n");
    assert_eq!(read(Some(2), Some(1)), "two\n");
}
