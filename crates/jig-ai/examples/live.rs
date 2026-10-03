//! Send one real command to a configured provider and print the reply.
//!
//!     cargo run -p jig-ai --example live              # the default provider
//!     cargo run -p jig-ai --example live -- ollama    # a provider by name

use jig_ai::{Config, PromptRequest};

fn main() -> anyhow::Result<()> {
    let config = Config::load(Config::user_path().as_deref())?;
    let provider_config = match std::env::args().nth(1) {
        Some(name) => config
            .providers
            .iter()
            .find(|p| p.name == name)
            .ok_or_else(|| anyhow::anyhow!("no provider named {name}"))?,
        None => config.default_provider(),
    };
    println!(
        "provider: {} ({})",
        provider_config.name, provider_config.model
    );
    let provider = provider_config.build()?;

    let text = "fn add(a: i32, b: i32) -> i32 {\n    a + b\n}\n";
    let request = PromptRequest {
        instruction: "Add concise doc comments to this code. Do not change the code itself.".into(),
        language: "rust".into(),
        file_name: Some("math.rs".into()),
        text: text.into(),
        target: 0..text.len() - 1,
    };
    let started = std::time::Instant::now();
    let reply = jig_ai::run(provider.as_ref(), &request)?;
    println!(
        "took: {:.1?}\nmessage: {}\nreplace:\n{}",
        started.elapsed(),
        reply.message,
        reply.replace
    );
    Ok(())
}
