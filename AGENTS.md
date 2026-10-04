# AGENTS.md

Jig is a macOS-first code editor (it also builds on Linux and Windows) where AI works through small commands run at the cursor, not through a chat or an agent somewhere else. You select code, press ⌘K, pick a command or type a prompt, and the change shows inline for you to accept or reject. Jig sends this file with every quick command, and OpenCode reads it for agent commands, so keep it short and current.

## The concept

**AI as a tool, not a colleague.** IDE refactorings are precise but fixed; agents are open-ended but take you away from the code. Jig uses the refactoring interaction with AI behind it. The name is from woodworking: a jig guides precise, repeatable cuts by hand.

Principles:

- **Near the code.** Every interaction is anchored to the cursor. No side panels.
- **You decide.** Every change is previewed and needs an explicit accept. Nothing reaches the disk unreviewed, from either lane.
- **Brief.** The AI answers in one sentence of at most 20 words, or not at all.
- **Keyboard first.** Few shortcuts, each obvious.
- **Clean and Mac-like.** The code is the interface. Restrained colour, floating panels, follows system light/dark.

### Two lanes

The command input (⌘K) has a **Quick | Agent** switch at the top. **Tab** flips it. Each lane has its own colour and icon everywhere it appears (switch, input, highlight, bubble tag), so you can always tell which one is running.

- **Quick** (theme primary/soft green, pencil). One model call that rewrites only the target: the selection (or current line), the cursor, or the whole file, per the command's `scope`. It sees the file and the nearest `AGENTS.md`, nothing else, and touches nothing else. Should take 2-5 s.
- **Agent** (theme info/orange, bot). Hands the task to OpenCode (`opencode serve`), which can read and edit the whole project with its own skills, commands and `AGENTS.md`. Each edit arrives as a diff *before* it's written and opens in its file's tab as a normal preview. Enter/Tab accepts (OpenCode writes it), Esc/⌘Z rejects (the agent is told and may retry). The run is a floating conversation at the code (`jig-commands/src/chat.rs`): what was asked, the agent's replies, each edit taken or rejected, and a reply box once the agent answers. It opens to the right of the code it's about when the window has room, below the cursor otherwise. Drag it by its header to move it (it then stays put) or by its corner to resize it. A reply continues the same OpenCode session. Esc with nothing under review stops the agent's turn; Esc again closes the conversation. Shell, web, subagents and its question tool are denied; it asks in its reply instead. 15-60 s.

A command can always run on the agent with `agent = true`; the switch then shows Agent with "set by this command".

### The command input

- ⌘K, the Command button in the title bar, or the sparkle button that appears just right of a selection while the code has focus (`workspace/selection_button.rs`; hidden while a command, preview or panel is open, and turned off with Settings > Appearance > Command button on selections).
- The typed text is listed first under **Prompt**, matching commands below under **Commands**. Enter takes a command only when the text clearly names it (its name or one of its words starts with the text); otherwise it runs the text as a prompt. ↑↓ pick either way. ⌘↩ saves the typed text as a command.
- **Notes**: `docs: terse, no examples` runs the command matching "docs" with the note "terse, no examples". The colon only counts when the text before it matches a command. Typing `:` on a command chosen with the arrows completes its name (`Simplify: `). Commands with `comment = "optional" | "required"` open a note step on Enter when no note was typed.
- The footer says what will happen, e.g. "Quick edit of the selection · sees this file, AGENTS.md".

### Review

A change goes into the buffer provisionally: highlighted, file read-only, one undo step. The model returns the whole new region, so Jig diffs it line by line (`jig-app/src/diff.rs`): only lines that differ are edited, only new lines are highlighted, and the bubble lists only lines actually removed. Tab/Enter accepts, Esc or ⌘Z rejects. A reply that changes nothing (e.g. an explanation) just shows its message.

## Layout

Rust workspace, edition 2024, toolchain pinned in `rust-toolchain.toml`.

- `crates/jig-app`: the binary (`Jig`). Window, workspace, tabs, file tree, Go to File (⌘P), Find in Files (⇧⌘F), menus, settings window, themes. `workspace/commands.rs` runs quick commands; `workspace/agent.rs` runs agent commands and their review; `agent.rs` owns the shared OpenCode server; `project.rs` finds the project root and `AGENTS.md`; `diff.rs` is the preview diff. `lsp.rs` runs language servers (not bundled; one that isn't installed is skipped) for ⌘-click go to definition and autocomplete, with `definitions.rs` guessing declarations and `completions.rs` offering words from the file when no server answers. `snippets.rs` adds Jig's snippets to the list, prefix (`for`) and postfix (`users.var`); `workspace/snippets.rs` fills a taken snippet in place by place with Tab and Shift-Tab. `workspace/status_bar.rs` shows cursor position, indentation (`indentation.rs`), encoding and language.
- Run configurations (IntelliJ-style): `run_configs.rs` loads the project's `.jig/run.toml` (`[[run]]` with `name`, `command` run through `/bin/sh`, optional `cwd`, `env`) and detects Cargo binaries/examples, `package.json` scripts, Go modules, Maven main classes (run with `exec:java`, debugged by their file) and Gradle's `run`; a file entry replaces a detected one of the same name. `run_output.rs` runs one in its own process group (Stop kills what it started), keeps ANSI colours and links `file:line:col`. `run_picker.rs` is the ⌃⌥R picker; `workspace/run.rs` has ⌃R run, ⌘F2 stop, ⌘J output panel, and the title-bar controls. Changed files are saved before a run. No stdin.
- Debugger (⌃D) over the Debug Adapter Protocol, generic like nvim-dap: debuggers are data. `assets/default-debuggers.toml` has the built-ins (Rust/C/C++ with `lldb-dap`, TypeScript/JavaScript with js-debug, Python with debugpy, Go with Delve, Java with java-debug in jdtls) and documents every field; the user's `~/.config/jig/debuggers.toml` adds more or replaces one by `key` (Run → Edit Debuggers). An entry says how to start the adapter (`adapter` on stdio or `server` with `${port}`, run in the configuration's folder; or `lsp` for one living in a language server Jig runs: `lsp_bundles` are passed to it in `initialize`, `lsp_start` is the `workspace/executeCommand` answering with the port, see `lsp_debug.rs`), which run configurations it takes (`commands` prefixes, `program` `extensions`, `executables`), how to check and `install` it (a shell command; Settings' Install button runs it into Jig's debuggers folder, whose `bin` is searched like the PATH), and a `launch` template with `${program}`, `${args}`, `${command}`, `${env}` and the like. `debuggers.rs` loads, merges and fills them in; the only per-debugger code is a few tool lookups (`${lldb_dap}` incl. Xcode and `lldb-dap-18`-style names, `${js_debug}`, `${node}`, `${lldb_rust_formatters}`) Java's values from jdtls (`${java_main_class}`, preferring the run configuration's `program` or the open file, `${java_project}`, `${java_class_paths}`, `${java_module_paths}`) and the Cargo builder (`build = "cargo"`: `cargo build --message-format=json-render-diagnostics`, debug the reported executable). `debug_target.rs` picks a configuration's debugger (run.toml `debugger`, else `program` extension, else longest `commands` match). Settings → Languages → Debugging turns each on (`[debugging] enabled`, default `rust`); only files of enabled languages get the breakpoint column. `dap.rs` is the client: stdio or TCP, several connections per session (js-debug runs the program in a child session it asks for with `startDebugging`), all messages tagged by connection on one channel; the session is a state machine over them, polled like run output. `workspace/breakpoints.rs`: ⌘F8 or a gutter click toggles a breakpoint, kept as a range decoration with a gutter dot (a vendored-editor patch) so it moves with edits; remembered in `~/.config/jig/breakpoints.json`. `workspace/debug.rs`: resume F9/⌥⌘R, step over F8, into F7, out ⇧F8, Stop ⌘F2; paused line highlighted; the output panel gets Debugger (call stack, variables tree) and Console views. Live tests use real debuggers (macOS may ask once to allow debugging): `cargo test -p jig-app tests::debug_live -- --ignored`, `lldb_dap_live`, and with `JIG_DEBUGGERS_DIR=<folder with them installed>`: `debug_typescript_live`, `debug_python_live`, `debug_go_live`, and with `JIG_LIVE_LSP=1 JIG_CACHE_DIR=<scratch>` too, `debug_java_live`; `built_in_installers_live` runs the install commands.
- Git, through the `git` command line (`git.rs`, no GUI code; the user's config, credential helpers and SSH agent apply, and git never gets a terminal to prompt on). `workspace/git.rs`: each tab's gutter has change bars against the last commit (green added, blue changed, red wedge for removed lines; a vendored-editor patch, `set_line_changes`), worked out again on every edit from the committed text, which is re-read when the window comes forward, on save, and after Jig's own git actions. Clicking a bar shows the old lines under the change, with Revert (one undo step). The branch in the title bar or ⌃⇧G opens the Git panel (`git_panel.rs`): fetch, pull, push (giving a new branch an upstream), a commit message, and staged and unstaged files to open, stage or unstage. Each file has a check (tracked changes start checked, untracked files don't): each section's header stages or unstages its checked files and nothing else (there's no Stage All, which would sweep in build folders), and ↩ or Commit commits exactly the checked files (`git commit --only`), leaving anything else staged as it was. Untracked folders are one entry; paths go to git on stdin. Git > Switch Branch… (`branch_picker.rs`) checks out a local or remote branch or creates one; changed files are saved first, and open tabs without unsaved changes reload afterwards.
- `crates/jig-commands`: command presets (TOML), the command palette, the add-command form, the reply bubble, the agent conversation, shared panel styling and lane colours (`surface.rs`).
- `crates/jig-editor`: `EditorHandle`, the only way the command layer touches the editor (text, selection, cursor, screen anchor, one-undo-step edits, highlights). Keeps the door open to replacing GPUI Kit's editor.
- `crates/jig-ai`: no GPUI, tested on its own. `Provider` trait with Anthropic and OpenAI-compatible implementations (between them nearly every provider; fields a server rejects by name are dropped and retried, `http.rs`), prompt building, reply parsing, and `agent.rs` (OpenCode client and unified-diff applier). `config_file.rs` edits `config.toml` keeping comments and holds the major-provider templates; `keys.rs` stores keys typed in Settings; `models.rs` lists a provider's models (chat models only); `agent::AgentTarget` hands the agent's provider to OpenCode as `OPENCODE_CONFIG_CONTENT`, named `jig-<provider>`.
- Providers in the app: `providers.rs` holds the config, keys and each provider's model list as a global that workspaces observe. Settings > Model connects the major providers (paste a key, or switch on a local server) and picks each lane's model from what the connected providers list. `agent.rs` restarts the shared OpenCode server when the agent's provider config changes; runs underway keep the old one.
- `assets/`: built-in commands (`default-commands.toml`), snippets (`default-snippets.toml`), Ember themes, file icons.

Per project: `.jig/run.toml` (run configurations). User files live in `~/.config/jig/`: `debuggers.toml` (own debuggers; same key replaces a built-in), `commands.toml` (own commands; same name replaces a built-in), `config.toml` (providers, and `quick` / `agent` as `provider/model`; the agent follows quick when unset; older `default` + `model` and `agent_model` still read), `keys.toml` (API keys, only where there's no system keychain), `settings.toml`, `snippets.toml` (own snippets; same name replaces a built-in).

## The quick-command contract

- Prompt (`jig-ai/src/prompt.rs`): instruction, optional note, language, file name, `AGENTS.md` as `<project_rules>`, and the file with the region marked `<<<SELECTION>>>…<<<END>>>` or `<<<CURSOR>>>`. Files over 24 KB are cut to whole lines around the region. The reply format is repeated after the file.
- Reply: `{"replace": "<new text for the region>", "message": "<≤20 words>"}`. Anthropic-format providers must return it through a forced `apply_edit` tool call; OpenAI-format ones use JSON mode. Anything that doesn't parse is an error and never changes the buffer.
- Thinking is off by default (`thinking: {type: disabled}` / `reasoning_effort: none`); with it on, reasoning models took 30-75 s and sometimes returned nothing. `thinking = true` on a provider turns it back on.
- Default model for both lanes: OpenCode Go `qwen3.8-flash` (Anthropic format, needs `OPENCODE_API_KEY` and an `x-opencode-session` header). API keys never go in `config.toml`: one typed in Settings goes in the system keychain (macOS Keychain, Windows Credential Manager; elsewhere a 0600 `keys.toml`) and wins over the `api_key_env` variable.

## Working on Jig

```sh
cargo run -p jig-app -- <file-or-folder>
cargo test                       # ~210 tests, including headless UI tests
cargo clippy --all-targets       # keep at zero warnings
cargo fmt
script/bundle-macos.sh           # builds target/Jig.app
```

- Live tests and tools that call real models are opt-in: `cargo test -p jig-app agent_live -- --ignored` (needs OpenCode), `cargo run -p jig-ai --example live -- --file <path> --lines 10-20` (times one quick command), `cargo run -p jig-ai --example agent -- <dir> "<prompt>"`. They cost money; don't run them in loops.
- Depend on `gpui-kit` only, pinned exactly (`=0.7.0`). GPUI distributions can't be mixed.
- UI tests open a real window on GPUI's test platform: `open(cx, path)`, then `step(cx, window, |window, cx| …)` to press keys and let effects settle. Type and press in separate steps when the list needs to refilter in between.
- Match the surrounding style: doc comments say why, in plain sentences; names over comments; small focused functions. User-facing text is short, plain and specific ("Save the file first, so the agent knows which project it's in.").
- Cross-platform: GPUI runs on macOS, Linux and Windows, and CI builds all three. Keep macOS-only touches behind `cfg!(target_os = "macos")` with a plain fallback, bind keys with `secondary-` (⌘ on macOS, Ctrl elsewhere), gate Unix APIs (`std::os::unix`, `libc`) with `#[cfg(unix)]`, and put caches and config where each OS expects them.
- New languages: enable the grammar feature in the root `Cargo.toml` and add it to `languages::BUNDLED`.
- When a feature lands or changes, update this file if it changes the concept, the layout or the contract.

## Not doing (for now)

Git beyond the panel (history, blame, merges, staging parts of files), terminal, plugins, LSP beyond go to definition and completion, project-wide replace, Jig's own editor element. Open: loading grammars at runtime, open-sourcing.
