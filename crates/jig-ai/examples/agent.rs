//! Run one agent request from the terminal, accepting every edit and
//! turning down everything else. OpenCode through Jig's own client:
//! `cargo run -p jig-ai --example agent -- <project dir> "<prompt>"`
//! or any ACP agent, by its command:
//! `cargo run -p jig-ai --example agent -- <project dir> "<prompt>" opencode acp`

use std::sync::Arc;

use jig_ai::acp::{AgentCommandLine, Connection};
use jig_ai::agent::{AgentEvent, AgentRequest, AgentServer, DEFAULT_MODEL, Permissions};
use jig_ai::harness::Backend;

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let directory = std::path::PathBuf::from(args.next().expect("project dir"));
    let prompt = args.next().expect("prompt");
    let command: Vec<String> = args.collect();
    let backend = match command.split_first() {
        None => Backend::OpenCode(Arc::new(AgentServer::start(None)?)),
        Some((program, rest)) => Backend::Acp(Connection::start(
            program,
            &AgentCommandLine {
                program: program.into(),
                args: rest.to_vec(),
                env: Vec::new(),
                path: None,
            },
        )?),
    };
    let session = backend.create(&directory, &prompt, &Permissions::default())?;
    let request = AgentRequest {
        directory,
        prompt,
        model: DEFAULT_MODEL.into(),
        command: None,
        permissions: Permissions::default(),
    };
    let replier = session.clone();
    session.run(&request, &|event| match event {
        AgentEvent::Step(step) => println!("· {step}"),
        AgentEvent::Edit(edit) => {
            let old = std::fs::read_to_string(&edit.path).unwrap_or_default();
            match edit.change.apply(&old) {
                Ok(new) => {
                    println!(
                        "edit {} ({} → {} bytes)",
                        edit.path.display(),
                        old.len(),
                        new.len()
                    );
                    if edit.change.written_by_jig() {
                        std::fs::write(&edit.path, new).unwrap();
                    }
                }
                Err(error) => println!("edit {}: {error}", edit.path.display()),
            }
            replier.reply(&edit.id, true, None).unwrap();
        }
        AgentEvent::Permission(request) => {
            println!("denied: {} {}", request.title, request.detail);
            replier.reply(&request.id, false, None).unwrap();
        }
        AgentEvent::Question(request) => {
            println!("declined {} questions", request.questions.len());
            replier.answer(&request.id, None).unwrap();
        }
        AgentEvent::Did(text) => println!("{text}"),
        AgentEvent::Commands(_) => {}
        AgentEvent::Done(text) => println!("done: {text}"),
    })
}
