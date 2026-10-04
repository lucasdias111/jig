//! Send one real command to a configured provider and print the reply.
//!
//!     cargo run -p jig-ai --example live                       # the quick model
//!     cargo run -p jig-ai --example live -- ollama/qwen3:8b    # provider/model
//!     cargo run -p jig-ai --example live -- --file src/x.rs --lines 10-20
//!                                     # the doc command on lines 10-20 of a real file

use jig_ai::{Config, PromptRequest};

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let flag = |name: &str| {
        args.iter()
            .position(|arg| arg == name)
            .and_then(|ix| args.get(ix + 1))
    };
    let name = args
        .iter()
        .enumerate()
        .find(|(ix, arg)| !arg.starts_with("--") && (*ix == 0 || !args[ix - 1].starts_with("--")))
        .map(|(_, arg)| arg);

    let config = Config::load(Config::user_path().as_deref())?;
    let provider_config = match name {
        Some(id) => Config {
            quick: Some(id.clone()),
            ..config
        }
        .quick()?,
        None => config.quick()?,
    };
    println!(
        "provider: {} ({})",
        provider_config.name, provider_config.model
    );
    let provider = provider_config.build()?;

    let file = flag("--file").map(std::fs::read_to_string).transpose()?;
    let text = file
        .as_deref()
        .unwrap_or("fn add(a: i32, b: i32) -> i32 {\n    a + b\n}\n");
    // Byte range of the selected lines, or the whole sample.
    let target = match flag("--lines").and_then(|lines| lines.split_once('-')) {
        Some((first, last)) => {
            let (first, last): (usize, usize) = (first.parse()?, last.parse()?);
            let starts: Vec<usize> = std::iter::once(0)
                .chain(text.match_indices('\n').map(|(ix, _)| ix + 1))
                .collect();
            starts[first - 1]..starts[last] - 1
        }
        None => 0..text.len() - 1,
    };
    let instruction = "Add concise doc comments to this code. Do not change the code itself.";
    let request = PromptRequest {
        instruction: instruction.into(),
        language: "rust".into(),
        file_name: Some("example.rs".into()),
        text: text.into(),
        target,
        comment: None,
        project_rules: None,
    };
    let started = std::time::Instant::now();
    let reply = if std::env::var_os("JIG_RAW").is_some() {
        let (system, user) = jig_ai::prompt::build(&request);
        let raw = provider.complete(&system, &user)?;
        println!("raw:\n{raw}\n---");
        jig_ai::response::parse(&raw, request.target_text())?
    } else {
        jig_ai::run(provider.as_ref(), &request)?
    };
    println!(
        "took: {:.1?}\nmessage: {}\nreplace:\n{}",
        started.elapsed(),
        reply.message,
        reply.replace
    );
    println!(
        "sent: {} bytes of file, {} selected; reply: {} bytes",
        text.len(),
        request.target.len(),
        reply.replace.len()
    );
    Ok(())
}
