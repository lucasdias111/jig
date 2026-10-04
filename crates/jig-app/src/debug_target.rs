//! What debugging a run configuration means: which debugger takes it, what
//! to build first, and the values its `launch` template is filled with.
//!
//! A configuration goes to the debugger `run.toml` names; else, for a
//! `program`, the one taking its extension (or native executables); else
//! the one whose `commands` its command starts with, the longest match
//! winning. A debugger with `build = "cargo"` builds `cargo run …` with
//! `cargo build` and debugs the executable Cargo reports.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::{Value, json};

use crate::debuggers::{Debugger, Registry, Vars};
use crate::run_configs::RunConfig;

#[derive(Clone, Debug)]
pub struct DebugPlan {
    pub debugger: Arc<Debugger>,
    pub build: Build,
    /// `${program}`: given, or the command's first word after the matched
    /// start. For a Cargo build, what it reports.
    program: Option<PathBuf>,
    args: Vec<String>,
    /// `${command}` and `${command_args}`: the command line, split.
    command: Vec<String>,
    /// The run configuration gave a `program`.
    from_program: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Build {
    None,
    /// A shell command, from run.toml's `build`.
    Shell(String),
    /// `cargo build` with these arguments, reading the executable from its
    /// JSON messages.
    Cargo(Vec<String>),
}

impl Build {
    /// The shell command that builds, if anything needs building.
    pub fn command(&self) -> Option<String> {
        match self {
            Build::None => None,
            Build::Shell(command) => Some(command.clone()),
            Build::Cargo(args) => {
                let mut command = "cargo build".to_string();
                for arg in args {
                    command.push(' ');
                    command.push_str(&crate::run_configs::shell_word(arg));
                }
                command.push_str(" --message-format=json-render-diagnostics");
                Some(command)
            }
        }
    }
}

impl DebugPlan {
    /// What to debug, when it's known before building.
    pub fn program(&self) -> Option<&Path> {
        self.program.as_deref()
    }

    /// What's being debugged, in words, for the console.
    pub fn describe(&self) -> String {
        match (&self.program, self.from_program || self.command.is_empty()) {
            (Some(program), true) => program.display().to_string(),
            _ => self.command.join(" "),
        }
    }

    /// The `launch` request's arguments, `program` being what the build
    /// made if there was one. Fills in tools too, which may run `rustc`.
    pub fn launch_arguments(
        &self,
        config: &RunConfig,
        root: &Path,
        program: Option<&Path>,
        extra: &[(String, Value)],
    ) -> Result<Value, String> {
        let program = program.or(self.program.as_deref());
        let env: serde_json::Map<String, Value> = config
            .env
            .iter()
            .map(|(k, v)| (k.clone(), Value::String(v.clone())))
            .collect();
        let env_list: Vec<String> = config.env.iter().map(|(k, v)| format!("{k}={v}")).collect();
        let (command, command_args) = match self.command.split_first() {
            Some((first, rest)) => (first.clone(), rest.to_vec()),
            None => (String::new(), Vec::new()),
        };
        let mut vars = Vars::new(&config.cwd);
        vars.set("name", json!(config.name))
            .set("cwd", json!(config.cwd))
            .set("root", json!(root))
            .set(
                "program",
                json!(program.map(Path::to_path_buf).unwrap_or_default()),
            )
            .set("args", json!(self.args))
            .set("command", json!(command))
            .set("command_args", json!(command_args))
            .set("env", Value::Object(env))
            .set("env_list", json!(env_list));
        for (name, value) in extra {
            vars.set(name, value.clone());
        }
        vars.expand(&self.debugger.launch_template(self.from_program))
    }
}

/// How to debug `config`: the plan, or why there's none.
pub fn plan_for(config: &RunConfig, registry: &Registry) -> Result<DebugPlan, String> {
    let named = match &config.debugger {
        Some(key) => Some(
            registry
                .get(key)
                .ok_or_else(|| format!("There's no debugger called “{key}”."))?,
        ),
        None => None,
    };
    if let Some(debug) = &config.debug {
        let extension = debug
            .program
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or_default();
        let debugger = named
            .or_else(|| {
                registry
                    .debuggers
                    .iter()
                    .find(|d| d.takes_extension(extension))
                    .or_else(|| registry.debuggers.iter().find(|d| d.takes_executables()))
                    .cloned()
            })
            .ok_or_else(|| no_debugger(config))?;
        let mut command = vec![debug.program.display().to_string()];
        command.extend(debug.args.iter().cloned());
        return Ok(DebugPlan {
            debugger,
            build: debug.build.clone().map_or(Build::None, Build::Shell),
            program: Some(debug.program.clone()),
            args: debug.args.clone(),
            command,
            from_program: true,
        });
    }
    let words =
        crate::run_configs::split_command(&config.command).ok_or_else(|| no_debugger(config))?;
    let matches = |debugger: &Debugger| {
        debugger
            .commands()
            .iter()
            .filter_map(|start| {
                let start: Vec<&str> = start.split_whitespace().collect();
                words
                    .iter()
                    .zip(&start)
                    .all(|(word, start)| word == start)
                    .then_some(start.len())
                    .filter(|len| *len <= words.len())
            })
            .max()
    };
    let (debugger, matched) = match named {
        Some(debugger) => {
            let matched = matches(&debugger).unwrap_or(1);
            (debugger, matched)
        }
        None => registry
            .debuggers
            .iter()
            .filter_map(|d| matches(d).map(|len| (d.clone(), len)))
            .max_by_key(|(_, len)| *len)
            .ok_or_else(|| no_debugger(config))?,
    };
    let rest = &words[matched..];
    if debugger.builds_with_cargo() && words.first().is_some_and(|w| w == "cargo") {
        let (build_args, args) = match rest.iter().position(|word| word == "--") {
            Some(split) => (&rest[..split], &rest[split + 1..]),
            None => (rest, &[][..]),
        };
        return Ok(DebugPlan {
            debugger,
            build: Build::Cargo(build_args.to_vec()),
            program: None,
            args: args.to_vec(),
            command: words.clone(),
            from_program: false,
        });
    }
    let program = rest.first().map(|first| {
        let path = config.cwd.join(first);
        if path.exists() {
            path
        } else {
            PathBuf::from(first)
        }
    });
    Ok(DebugPlan {
        debugger,
        build: Build::None,
        program,
        args: rest.iter().skip(1).cloned().collect(),
        command: words.clone(),
        from_program: false,
    })
}

fn no_debugger(config: &RunConfig) -> String {
    format!(
        "No debugger takes “{}”. Name one with `debugger` in its run.toml entry, or add \
         one that does in debuggers.toml.",
        config.name
    )
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::run_configs::{DebugProgram, Source};

    fn registry() -> Arc<Registry> {
        crate::debuggers::registry()
    }

    fn config(command: &str) -> RunConfig {
        RunConfig {
            name: "t".into(),
            command: command.into(),
            cwd: "/p".into(),
            env: Vec::new(),
            source: Source::File,
            debug: None,
            debugger: None,
        }
    }

    fn launch(config: &RunConfig) -> Value {
        plan_for(config, &registry())
            .unwrap()
            .launch_arguments(config, Path::new("/p"), None, &[])
            .unwrap()
    }

    #[test]
    fn cargo_run_builds_with_cargo() {
        let plan = plan_for(
            &config("cargo run -p jig-app --bin Jig -- . -v"),
            &registry(),
        )
        .unwrap();
        assert_eq!(plan.debugger.key, "rust");
        assert_eq!(
            plan.build.command().unwrap(),
            "cargo build -p jig-app --bin Jig --message-format=json-render-diagnostics"
        );
        let args = plan
            .launch_arguments(
                &config("x"),
                Path::new("/p"),
                Some(Path::new("/p/target/Jig")),
                &[],
            )
            .unwrap();
        assert_eq!(args["program"], "/p/target/Jig");
        assert_eq!(args["args"], json!([".", "-v"]));
        assert_eq!(args["type"], "lldb-dap");
    }

    #[test]
    fn commands_go_to_the_debugger_they_start_like() {
        let npm = launch(&config("npm run dev"));
        assert_eq!(npm["runtimeExecutable"], "npm");
        assert_eq!(npm["runtimeArgs"], json!(["run", "dev"]));

        let python = launch(&config("python3 app.py --fast"));
        assert_eq!(python["type"], "debugpy");
        assert_eq!(python["program"], "app.py");
        assert_eq!(python["args"], json!(["--fast"]));

        let go = launch(&config("go run ."));
        assert_eq!(go["mode"], "debug");
        // `/p` doesn't exist, so it stays as written; the adapter runs there.
        assert_eq!(go["program"], ".");
        assert_eq!(go["outputMode"], "remote");

        assert!(plan_for(&config("make run"), &registry()).is_err());
        assert!(plan_for(&config("cargo run && echo done"), &registry()).is_err());
    }

    #[test]
    fn programs_go_by_extension_then_to_native_debugging() {
        let script = RunConfig {
            debug: Some(DebugProgram {
                program: "/p/src/main.ts".into(),
                args: vec!["--fast".into()],
                build: None,
            }),
            env: vec![("PORT".into(), "8080".into())],
            ..config("node src/main.ts")
        };
        let args = launch(&script);
        assert_eq!(args["program"], "/p/src/main.ts");
        assert_eq!(args["args"], json!(["--fast"]));
        assert_eq!(args["env"], json!({"PORT": "8080"}));

        let native = RunConfig {
            debug: Some(DebugProgram {
                program: "/p/build/app".into(),
                args: Vec::new(),
                build: Some("make".into()),
            }),
            ..config("make run")
        };
        let plan = plan_for(&native, &registry()).unwrap();
        assert_eq!(plan.debugger.key, "rust");
        assert_eq!(plan.build, Build::Shell("make".into()));
    }

    #[test]
    fn run_toml_can_name_the_debugger() {
        let named = RunConfig {
            debugger: Some("python".into()),
            ..config("./manage.py runserver")
        };
        let plan = plan_for(&named, &registry()).unwrap();
        assert_eq!(plan.debugger.key, "python");
        let missing = RunConfig {
            debugger: Some("cobol".into()),
            ..config("x")
        };
        assert!(
            plan_for(&missing, &registry())
                .unwrap_err()
                .contains("cobol")
        );
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
