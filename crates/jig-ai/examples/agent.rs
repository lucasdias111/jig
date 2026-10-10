//! Run one agent command from the terminal, accepting every edit and
//! turning down everything else:
//! `cargo run -p jig-ai --example agent -- <project dir> "<prompt>"`

use std::sync::Arc;

use jig_ai::agent::{
    AgentEvent, AgentRequest, AgentServer, AgentSession, DEFAULT_MODEL, Permissions, apply_diff,
};

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let directory = std::path::PathBuf::from(args.next().expect("project dir"));
    let prompt = args.next().expect("prompt");
    let server = Arc::new(AgentServer::start(None)?);
    let session = AgentSession::create(server, &directory, &prompt, &Permissions::default())?;
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
            match apply_diff(&old, &edit.diff) {
                Ok(new) => println!(
                    "edit {} ({} → {} bytes)",
                    edit.path.display(),
                    old.len(),
                    new.len()
                ),
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
        AgentEvent::Done(text) => println!("done: {text}"),
    })
}
