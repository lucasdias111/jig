//! Send one real command to a configured provider and print the reply.
//!
//!     cargo run -p jig-ai --example live                       # the default provider
//!     cargo run -p jig-ai --example live -- ollama             # a provider by name
//!     cargo run -p jig-ai --example live -- --explore          # let it read this repo

use jig_ai::{Config, ProjectTools, PromptRequest};

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let explore = args.iter().any(|arg| arg == "--explore");
    let name = args.iter().find(|arg| !arg.starts_with("--"));

    let config = Config::load(Config::user_path().as_deref())?;
    let provider_config = match name {
        Some(name) => config
            .providers
            .iter()
            .find(|p| &p.name == name)
            .ok_or_else(|| anyhow::anyhow!("no provider named {name}"))?,
        None => config.default_provider(),
    };
    println!(
        "provider: {} ({})",
        provider_config.name, provider_config.model
    );
    let provider = provider_config.build()?;

    let (text, instruction) = if explore {
        (
            "fn example_request() -> PromptRequest {\n    todo!()\n}\n",
            "Implement this. PromptRequest is defined elsewhere in the project; fill in every field with sensible example values.",
        )
    } else {
        (
            "fn add(a: i32, b: i32) -> i32 {\n    a + b\n}\n",
            "Add concise doc comments to this code. Do not change the code itself.",
        )
    };
    let request = PromptRequest {
        instruction: instruction.into(),
        language: "rust".into(),
        file_name: Some("example.rs".into()),
        text: text.into(),
        target: 0..text.len() - 1,
        comment: None,
        project_rules: None,
    };
    let started = std::time::Instant::now();
    let reply = if explore {
        let tools = ProjectTools::new(&std::env::current_dir()?)?;
        jig_ai::run_exploring(provider.as_ref(), &request, &tools, &|step| {
            println!("  {:>5.1?}  {step}", started.elapsed())
        })?
    } else {
        jig_ai::run(provider.as_ref(), &request)?
    };
    println!(
        "took: {:.1?}\nmessage: {}\nreplace:\n{}",
        started.elapsed(),
        reply.message,
        reply.replace
    );
    Ok(())
}
