//! What debugging a run configuration means: which debugger, what to build,
//! and what to start under it.
//!
//! - `cargo run …` debugs under lldb: Jig builds the same target with
//!   `cargo build` and debugs the executable Cargo reports.
//! - Commands that run Node, such as `npm run dev`, `node app.js` or
//!   `tsx src/main.ts`, debug under js-debug, which attaches to every Node
//!   process the command starts.
//! - Anything else needs `program` in `run.toml`: a `.js` or `.ts` file
//!   debugs under js-debug, any other under lldb.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde_json::{Value, json};

use crate::debuggers::Kind;
use crate::run_configs::{RunConfig, Source};

/// Programs whose commands run Node, so js-debug can debug them.
const NODE_COMMANDS: &[&str] = &[
    "node", "npm", "npx", "pnpm", "yarn", "tsx", "ts-node", "nodemon", "vite", "next",
];
/// Script files that run on Node; Node 23.6+ runs TypeScript as it is.
const NODE_EXTENSIONS: &[&str] = &["js", "mjs", "cjs", "ts", "mts", "cts"];

#[derive(Clone, Debug, PartialEq)]
pub enum DebugTarget {
    /// Build with `cargo build <build_args>`, then debug what it built.
    Cargo {
        build_args: Vec<String>,
        args: Vec<String>,
    },
    /// Run `build` if given, then debug the executable `program`.
    Program {
        build: Option<String>,
        program: PathBuf,
        args: Vec<String>,
    },
    /// Run `build` if given, then debug a script, or a command, on Node.
    Node {
        build: Option<String>,
        launch: NodeLaunch,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub enum NodeLaunch {
    /// A `.js` or `.ts` file, with its arguments.
    Script { program: PathBuf, args: Vec<String> },
    /// A command line, such as `npm run dev`, as words.
    Command(Vec<String>),
}

impl DebugTarget {
    pub fn kind(&self) -> Kind {
        match self {
            DebugTarget::Cargo { .. } | DebugTarget::Program { .. } => Kind::Lldb,
            DebugTarget::Node { .. } => Kind::JsDebug,
        }
    }

    /// The shell command that builds it, if anything needs building.
    pub fn build_command(&self) -> Option<String> {
        match self {
            DebugTarget::Cargo { build_args, .. } => {
                let mut command = "cargo build".to_string();
                for arg in build_args {
                    command.push(' ');
                    command.push_str(&crate::run_configs::shell_word(arg));
                }
                command.push_str(" --message-format=json-render-diagnostics");
                Some(command)
            }
            DebugTarget::Program { build, .. } | DebugTarget::Node { build, .. } => build.clone(),
        }
    }

    /// The executable to debug, when it's known before building.
    pub fn executable(&self) -> Option<PathBuf> {
        match self {
            DebugTarget::Program { program, .. } => Some(program.clone()),
            DebugTarget::Node {
                launch: NodeLaunch::Script { program, .. },
                ..
            } => Some(program.clone()),
            DebugTarget::Node { .. } => Some(PathBuf::from("node")),
            DebugTarget::Cargo { .. } => None,
        }
    }

    /// The `launch` request's arguments for debugging it as `config` would
    /// run it, `executable` being what the build made. For lldb this asks
    /// `rustc` where its sysroot is.
    pub fn launch_arguments(&self, config: &RunConfig, executable: &Path) -> Value {
        match self {
            DebugTarget::Cargo { args, .. } | DebugTarget::Program { args, .. } => {
                let env: Vec<String> = config.env.iter().map(|(k, v)| format!("{k}={v}")).collect();
                json!({
                    "name": config.name,
                    "type": "lldb-dap",
                    "request": "launch",
                    "program": executable,
                    "args": args,
                    "cwd": config.cwd,
                    "env": env,
                    "stopOnEntry": false,
                    "initCommands": rust_init_commands(&config.cwd),
                })
            }
            DebugTarget::Node { launch, .. } => {
                let env: serde_json::Map<String, Value> = config
                    .env
                    .iter()
                    .map(|(k, v)| (k.clone(), Value::String(v.clone())))
                    .collect();
                let mut arguments = json!({
                    "name": config.name,
                    "type": "pwa-node",
                    "request": "launch",
                    "cwd": config.cwd,
                    "env": env,
                    "console": "internalConsole",
                    "sourceMaps": true,
                    // Stepping into Node's own code is never wanted.
                    "skipFiles": ["<node_internals>/**"],
                });
                match launch {
                    NodeLaunch::Script { program, args } => {
                        arguments["program"] = json!(program);
                        arguments["args"] = json!(args);
                    }
                    NodeLaunch::Command(words) => {
                        arguments["runtimeExecutable"] = json!(words[0]);
                        arguments["runtimeArgs"] = json!(words[1..]);
                    }
                }
                arguments
            }
        }
    }
}

/// How to debug `config`, if Jig knows.
pub fn target_for(config: &RunConfig) -> Option<DebugTarget> {
    if let Some(debug) = &config.debug {
        let is_script = debug
            .program
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| NODE_EXTENSIONS.contains(&e));
        return Some(if is_script {
            DebugTarget::Node {
                build: debug.build.clone(),
                launch: NodeLaunch::Script {
                    program: debug.program.clone(),
                    args: debug.args.clone(),
                },
            }
        } else {
            DebugTarget::Program {
                build: debug.build.clone(),
                program: debug.program.clone(),
                args: debug.args.clone(),
            }
        });
    }
    let words = split_command(&config.command)?;
    if let [cargo, run, rest @ ..] = words.as_slice()
        && cargo == "cargo"
        && run == "run"
    {
        let (build_args, args) = match rest.iter().position(|word| word == "--") {
            Some(split) => (&rest[..split], &rest[split + 1..]),
            None => (rest, &[][..]),
        };
        return Some(DebugTarget::Cargo {
            build_args: build_args.to_vec(),
            args: args.to_vec(),
        });
    }
    let runs_node = config.source == Source::Npm || NODE_COMMANDS.contains(&words[0].as_str());
    runs_node.then_some(DebugTarget::Node {
        build: None,
        launch: NodeLaunch::Command(words),
    })
}

/// `command` split into words as `sh` would, or `None` when it does more
/// than run one program: pipes, `&&`, variables, redirections.
pub fn split_command(command: &str) -> Option<Vec<String>> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut in_word = false;
    let mut chars = command.chars();
    while let Some(c) = chars.next() {
        match c {
            '\'' => {
                in_word = true;
                loop {
                    match chars.next()? {
                        '\'' => break,
                        c => word.push(c),
                    }
                }
            }
            '"' => {
                in_word = true;
                loop {
                    match chars.next()? {
                        '"' => break,
                        '\\' => word.push(chars.next()?),
                        '$' | '`' => return None,
                        c => word.push(c),
                    }
                }
            }
            '\\' => {
                in_word = true;
                word.push(chars.next()?);
            }
            c if c.is_whitespace() => {
                if in_word {
                    words.push(std::mem::take(&mut word));
                    in_word = false;
                }
            }
            '|' | '&' | ';' | '<' | '>' | '$' | '`' | '(' | ')' | '*' | '?' => return None,
            c => {
                in_word = true;
                word.push(c);
            }
        }
    }
    if in_word {
        words.push(word);
    }
    (!words.is_empty()).then_some(words)
}

/// The executable a line of `cargo build --message-format=json` output
/// reports building, if it reports one.
pub fn cargo_executable(line: &str) -> Option<PathBuf> {
    let message: Value = serde_json::from_str(line).ok()?;
    if message["reason"] != "compiler-artifact" {
        return None;
    }
    message["executable"].as_str().map(PathBuf::from)
}

/// LLDB commands that load Rust's pretty-printers, so a `Vec` or `String`
/// shows its contents. Empty when there's no Rust toolchain.
fn rust_init_commands(cwd: &Path) -> Vec<String> {
    let Some(sysroot) = Command::new("rustc")
        .args(["--print", "sysroot"])
        .current_dir(cwd)
        .env("PATH", crate::lsp::search_path())
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| PathBuf::from(String::from_utf8_lossy(&output.stdout).trim()))
    else {
        return Vec::new();
    };
    // Recent toolchains register everything from `lldb_lookup.py`; older
    // ones also have a commands file to source.
    let etc = sysroot.join("lib/rustlib/etc");
    let lookup = etc.join("lldb_lookup.py");
    if !lookup.is_file() {
        return Vec::new();
    }
    let mut commands = vec![format!("command script import \"{}\"", lookup.display())];
    let extra = etc.join("lldb_commands");
    if extra.is_file() {
        commands.push(format!("command source -s 0 \"{}\"", extra.display()));
    }
    commands
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::run_configs::DebugProgram;

    fn config(command: &str) -> RunConfig {
        RunConfig {
            name: "t".into(),
            command: command.into(),
            cwd: "/p".into(),
            env: Vec::new(),
            source: Source::File,
            debug: None,
        }
    }

    #[test]
    fn cargo_run_debugs_what_cargo_build_builds() {
        let target = target_for(&config("cargo run -p jig-app --bin Jig -- . --verbose")).unwrap();
        assert_eq!(
            target,
            DebugTarget::Cargo {
                build_args: vec!["-p".into(), "jig-app".into(), "--bin".into(), "Jig".into()],
                args: vec![".".into(), "--verbose".into()],
            }
        );
        assert_eq!(
            target.build_command().unwrap(),
            "cargo build -p jig-app --bin Jig --message-format=json-render-diagnostics"
        );
    }

    #[test]
    fn other_commands_need_a_program() {
        assert_eq!(target_for(&config("make run")), None);
        assert_eq!(target_for(&config("cargo run && echo done")), None);
        let with_program = RunConfig {
            debug: Some(DebugProgram {
                program: "/p/build/app".into(),
                args: vec!["-v".into()],
                build: Some("make".into()),
            }),
            ..config("make run")
        };
        assert_eq!(
            target_for(&with_program),
            Some(DebugTarget::Program {
                build: Some("make".into()),
                program: "/p/build/app".into(),
                args: vec!["-v".into()],
            })
        );
    }

    #[test]
    fn node_commands_and_scripts_debug_on_node() {
        let target = target_for(&config("npm run dev")).unwrap();
        assert_eq!(target.kind(), Kind::JsDebug);
        let launch = target.launch_arguments(&config("npm run dev"), Path::new("node"));
        assert_eq!(launch["runtimeExecutable"], "npm");
        assert_eq!(launch["runtimeArgs"], json!(["run", "dev"]));
        assert_eq!(launch["type"], "pwa-node");

        let script = RunConfig {
            debug: Some(DebugProgram {
                program: "/p/src/main.ts".into(),
                args: vec!["--fast".into()],
                build: None,
            }),
            env: vec![("PORT".into(), "8080".into())],
            ..config("node src/main.ts")
        };
        let target = target_for(&script).unwrap();
        assert_eq!(target.kind(), Kind::JsDebug);
        let launch = target.launch_arguments(&script, Path::new("/p/src/main.ts"));
        assert_eq!(launch["program"], "/p/src/main.ts");
        assert_eq!(launch["args"], json!(["--fast"]));
        assert_eq!(launch["env"], json!({"PORT": "8080"}));
    }

    #[test]
    fn commands_split_like_sh() {
        assert_eq!(
            split_command(r#"cargo run -- "two words" 'it''s' a\ b"#).unwrap(),
            ["cargo", "run", "--", "two words", "its", "a b"]
        );
        assert_eq!(split_command("echo $HOME"), None);
        assert_eq!(split_command("ls | wc"), None);
        assert_eq!(split_command("echo 'open"), None);
    }

    #[test]
    fn the_executable_comes_from_the_artifact_message() {
        let line = r#"{"reason":"compiler-artifact","target":{"kind":["bin"]},"executable":"/p/target/debug/app"}"#;
        assert_eq!(cargo_executable(line), Some("/p/target/debug/app".into()));
        let lib = r#"{"reason":"compiler-artifact","target":{"kind":["lib"]},"executable":null}"#;
        assert_eq!(cargo_executable(lib), None);
        assert_eq!(
            cargo_executable(r#"{"reason":"build-finished","success":true}"#),
            None
        );
    }
}
