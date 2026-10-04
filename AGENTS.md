# AGENTS.md

Jig is a macOS-first code editor where AI works through small commands run at the cursor, not through a chat or an agent somewhere else. You select code, press ⌘K, pick a command or type a prompt, and the change shows inline for you to accept or reject. Jig sends this file with every quick command, and OpenCode reads it for agent commands, so keep it short and current.

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

- **Quick** (theme primary/orange, pencil). One model call that rewrites only the target: the selection (or current line), the cursor, or the whole file, per the command's `scope`. It sees the file and the nearest `AGENTS.md`, nothing else, and touches nothing else. Should take 2-5 s.
- **Agent** (theme info/teal, bot). Hands the task to OpenCode (`opencode serve`), which can read and edit the whole project with its own skills, commands and `AGENTS.md`. Each edit arrives as a diff *before* it's written and opens in its file's tab as a normal preview. Enter/Tab accepts (OpenCode writes it), Esc/⌘Z rejects (the agent is told and may retry). Esc with nothing under review stops the agent. Shell, web, subagents and questions are denied. 15-60 s.

A command can always run on the agent with `agent = true`; the switch then shows Agent with "set by this command".

### The command input

- The typed text is listed first under **Prompt**, matching commands below under **Commands**. Enter takes a command only when the text clearly names it (its name or one of its words starts with the text); otherwise it runs the text as a prompt. ↑↓ pick either way. ⌘↩ saves the typed text as a command.
- **Notes**: `docs: terse, no examples` runs the command matching "docs" with the note "terse, no examples". The colon only counts when the text before it matches a command. Typing `:` on a command chosen with the arrows completes its name (`Simplify: `). Commands with `comment = "optional" | "required"` open a note step on Enter when no note was typed.
- The footer says what will happen, e.g. "Quick edit of the selection · sees this file, AGENTS.md".

### Review

A change goes into the buffer provisionally: highlighted, file read-only, one undo step. The model returns the whole new region, so Jig diffs it line by line (`jig-app/src/diff.rs`): only lines that differ are edited, only new lines are highlighted, and the bubble lists only lines actually removed. Tab/Enter accepts, Esc or ⌘Z rejects. A reply that changes nothing (e.g. an explanation) just shows its message.

## Layout

Rust workspace, edition 2024, toolchain pinned in `rust-toolchain.toml`.

- `crates/jig-app`: the binary (`Jig`). Window, workspace, tabs, file tree, Go to File (⌘P), Find in Files (⇧⌘F), menus, settings window, themes. `workspace/commands.rs` runs quick commands; `workspace/agent.rs` runs agent commands and their review; `agent.rs` owns the shared OpenCode server; `project.rs` finds the project root and `AGENTS.md`; `diff.rs` is the preview diff.
- `crates/jig-commands`: command presets (TOML), the command palette, the add-command form, the reply bubble, shared panel styling and lane colours (`surface.rs`).
- `crates/jig-editor`: `EditorHandle`, the only way the command layer touches the editor (text, selection, cursor, screen anchor, one-undo-step edits, highlights). Keeps the door open to replacing GPUI Kit's editor.
- `crates/jig-ai`: no GPUI, tested on its own. `Provider` trait with Anthropic and OpenAI-compatible implementations, prompt building, reply parsing, and `agent.rs` (OpenCode client and unified-diff applier).
- `assets/`: built-in commands (`default-commands.toml`), Ember themes, file icons.

User files live in `~/.config/jig/`: `commands.toml` (own commands; same name replaces a built-in), `config.toml` (providers, `default`, `agent_model`), `settings.toml`.

## The quick-command contract

- Prompt (`jig-ai/src/prompt.rs`): instruction, optional note, language, file name, `AGENTS.md` as `<project_rules>`, and the file with the region marked `<<<SELECTION>>>…<<<END>>>` or `<<<CURSOR>>>`. Files over 24 KB are cut to whole lines around the region. The reply format is repeated after the file.
- Reply: `{"replace": "<new text for the region>", "message": "<≤20 words>"}`. Anthropic-format providers must return it through a forced `apply_edit` tool call; OpenAI-format ones use JSON mode. Anything that doesn't parse is an error and never changes the buffer.
- Thinking is off by default (`thinking: {type: disabled}` / `reasoning_effort: none`); with it on, reasoning models took 30-75 s and sometimes returned nothing. `thinking = true` on a provider turns it back on.
- Default provider: OpenCode Go `qwen3.8-flash` (Anthropic format, needs `OPENCODE_API_KEY` and an `x-opencode-session` header). API keys only ever come from environment variables named in `config.toml`.

## Working on Jig

```sh
cargo run -p jig-app -- <file-or-folder>
cargo test                       # ~170 tests, including headless UI tests
cargo clippy --all-targets       # keep at zero warnings
cargo fmt
script/bundle-macos.sh           # builds target/Jig.app
```

- Live tests and tools that call real models are opt-in: `cargo test -p jig-app agent_live -- --ignored` (needs OpenCode), `cargo run -p jig-ai --example live -- --file <path> --lines 10-20` (times one quick command), `cargo run -p jig-ai --example agent -- <dir> "<prompt>"`. They cost money; don't run them in loops.
- Depend on `gpui-kit` only, pinned exactly (`=0.7.0`). GPUI distributions can't be mixed.
- UI tests open a real window on GPUI's test platform: `open(cx, path)`, then `step(cx, window, |window, cx| …)` to press keys and let effects settle. Type and press in separate steps when the list needs to refilter in between.
- Match the surrounding style: doc comments say why, in plain sentences; names over comments; small focused functions. User-facing text is short, plain and specific ("Save the file first, so the agent knows which project it's in.").
- New languages: enable the grammar feature in the root `Cargo.toml` and add it to `languages::BUNDLED`.
- When a feature lands or changes, update this file if it changes the concept, the layout or the contract.

## Not doing (for now)

LSP, git UI, terminal, plugins, project-wide replace, Jig's own editor element. Open: loading grammars at runtime, moving API keys to the Keychain (an app opened from Finder doesn't see shell variables), open-sourcing.
