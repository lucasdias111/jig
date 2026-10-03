# Jig: Plan

Oct 3, 2026 · @Lucas

Jig is a code editor built to give developers control back. AI does the work through small, contained commands run right at the cursor, instead of in an agent that lives somewhere else.

## Status

*Updated Oct 3, 2026.*

**Milestones M0–M5 are done.** The full concept works end to end: open a file, select code, press Cmd+K, pick or type a command, review the change inline, accept or reject it. Since then Jig has also gained OneNord themes, a macOS menu bar, commands you can add from inside Jig, notes on commands, `JIG.md` project rules, Qwen3.8 Flash as the default model, project exploration for commands, a file tree for browsing the open folder, tabs, and a Settings window.

| Area | State |
|---|---|
| M0 Spike | ✅ Done. GPUI Kit's editor exposes everything Jig needs, so the cursor-coordinate risk is gone. |
| M1 Open and edit | ✅ Done. Rust only, atomic save, unsaved marker and discard prompts. |
| M2 Floating command input | ✅ Done. Fuzzy filter; typed text runs as a custom command. |
| M3 AI round trip | ✅ Done. Verified live against OpenCode Go. |
| M4 Apply and review | ✅ Done. Provisional apply, Tab/Enter to accept, Esc or Cmd+Z to reject, one undo step. |
| M5 Polish | ✅ Done. Fade-and-slide instead of scale, which GPUI can't do. |
| Themes | ✅ OneNord light and dark, following the system. |
| macOS menu bar | ✅ Jig, File, Edit and View menus; `script/bundle-macos.sh` builds `Jig.app`. |
| Custom commands | ✅ Add Command form (⇧⌘K), Cmd+Enter to save typed text as a command, Edit Commands File. |
| Command notes | ✅ `comment` setting per command; Tab adds a one-off note to any command. |
| Project rules | ✅ The nearest `JIG.md` is sent with every command. |
| Project exploration | ✅ Commands with `explore = true`, or any command with ⌘E, may read the project (read-only, at most 8 tool calls). Verified live with Qwen and GLM, about 19 s per exploring command. |
| Folder and file tree | ✅ Open… takes a file or a folder. The sidebar lists the whole project lazily, folders first, gitignored entries dimmed. ⌘B toggles it, ⇧⌘E moves focus in and out, arrows and Enter navigate, and its edge drags to resize. |
| Tabs | ✅ Each tab has its own buffer, unsaved state and undo history. ⌘N, ⌘W, ⌘1–9, ⇧⌘[ / ⇧⌘], ⌃Tab. The strip shows only with two or more tabs. |
| Settings | ✅ ⌘, opens a Settings window. Appearance (theme, code font size, line numbers, wrap, indent guides, whitespace), Commands (turn commands on or off, add or edit them) and Model (pick the provider, see whether its key is set). Saved to `~/.config/jig/settings.toml`. |
| Cmd+P | 📋 Planned next. |

The code is a Rust workspace with four crates, about 100 tests (including headless UI tests) and a commit per milestone.

## Background

AI coding tools have moved away from the code. The common setup today is code in the editor and an agent in a terminal or a separate chat panel. You describe what you want in prose, the agent goes off, edits files you are not looking at, and you review a diff afterwards.

That works for large, loosely defined tasks. For everyday work it has real costs:

- **You lose your place.** Attention moves from the code to a conversation and back again.
- **You lose ownership.** Changes arrive in bulk, often wider than asked, and you end up reviewing code you did not shape.
- **You lose precision.** Prose is a blunt way to describe a change to a specific function, block or line.
- **You get chatter.** Agents explain, summarise and ask follow-ups when you wanted the edit.

IDEs like IntelliJ already showed a better model for structured changes: select code, run a refactoring, see the result in place. Those refactorings are precise and predictable, but limited to what someone hard-coded.

Jig combines the two. The interaction model of IDE refactorings, with AI behind it so the set of commands is open-ended. You stay in the code, you choose exactly what is touched, and the AI behaves like a tool rather than a colleague.

The name comes from woodworking. A jig is a guide that makes precise, repeatable cuts possible by hand. Jig's preset commands play the same role: the craftsperson stays in control, the jig makes the work fast and exact.

## The concept

Every AI interaction in Jig happens in one place: a small floating input next to the cursor.

1. Place the cursor or select some code.
2. Press the command shortcut. A floating input opens right below the cursor or selection.
3. Pick a preset command (Create controller, Add docs, Extract method) or type a custom instruction.
4. The AI returns the edit and a reply of about 20 words in a small floating message window.
5. The change shows inline. Accept it or throw it away, and you are back in the code.

**Jig is**

- A clean, fast editor for reading and writing code
- A set of precise AI commands scoped to a selection, the cursor or the current file
- Local-model or cloud-model friendly

**Jig is not**

- An agent that plans, explores the repo or runs for minutes on its own
- A chat panel or a sparring partner
- A full IDE replacement, at least not in v1

## Design principles

- **Near the code.** Every interaction is anchored to the cursor. Nothing pulls your eyes to a side panel.
- **Small and contained.** A command touches what you selected and nothing else.
- **You decide.** Every change is previewed and needs an explicit accept.
- **Brief.** The AI answers in one short sentence or not at all.
- **Keyboard first.** Everything reachable without the mouse.

### Aesthetic

Super clean and Mac-like. The code is the interface.

- Hidden or transparent titlebar, no toolbars, generous padding
- SF Pro for UI and SF Mono for code on macOS; Inter and JetBrains Mono as fallbacks on Linux
- Floating windows with rounded corners, soft shadows and quick fade and scale animations
- Follows system light and dark mode, one carefully tuned theme for each
- Restrained colour: syntax highlighting carries most of it

## Technical approach

Jig is written in Rust on GPUI, Zed's GPU-accelerated UI framework (Apache-2.0). GPUI gives fast rendering, text shaping, layout and windowing on macOS, Linux and Windows.

### Borrow the editor now, own it later

*Status: in use. GPUI Kit 0.7.0's editor exposes the cursor's and any range's screen position, the selection, an atomic replace that is one undo step, and range highlights. The adapter is `jig-editor`'s `EditorHandle`.*

The commands are what Jig is testing. The text editor is the means. For v1, the editing surface is GPUI Kit's code editor component, which already handles tree-sitter highlighting, selection and multi-cursor. All effort goes into the command layer.

The command layer only talks to the editor through a small interface: buffer text, selection range, cursor position, cursor screen coordinates, and "apply this edit as one undo step". That keeps the door open to replacing the component with Jig's own editor element later, without touching any AI code.

### GPUI distribution

The official gpui crate on crates.io lags far behind upstream, and GPUI distributions cannot be mixed in one dependency graph. Jig depends on `gpui-kit` and nothing else GPUI-related, pinned to an exact version.

*Status: pinned to `gpui-kit = "=0.7.0"`, which brings `gpui` 0.3.7. Rust highlighting comes from its `tree-sitter-rust` feature.*

### Crate layout

- `jig-app`: window, workspace, file tree sidebar, keybindings, theming
- `jig-editor`: the editor adapter (wraps GPUI Kit's component behind the interface above)
- `jig-commands`: preset loading, filtering, the floating input and reply UI, diff preview
- `jig-ai`: the `Provider` trait, prompt building, response parsing, read-only project tools (no GPUI, so it is tested on its own)

### AI providers

One `Provider` trait, two implementations at first:

- Anthropic API
- OpenAI-compatible endpoint, which covers Ollama and llama.cpp for local models

Requests run off the main thread. The UI shows a small spinner in the floating window until the result is ready.

*Status: providers are configured in `~/.config/jig/config.toml`, with API keys read only from environment variables. The built-in providers are:*

- *`opencode-qwen`: OpenCode Go, `qwen3.8-flash`, Anthropic format. **The default**, about 5 s per command.*
- *`opencode-glm`: OpenCode Go, `glm-5.3-flash`, OpenAI format. About 10 s per command.*
- *`claude`: the Anthropic API.*
- *`ollama`: local `qwen2.5-coder:7b`.*

*OpenCode Go needs an `x-opencode-session` header and a client user agent, which Jig sends. Models that only speak OpenAI's `/responses` API (GPT, Grok) aren't supported. The waiting state shows the command name, a shimmer, elapsed seconds and a progress bar, and tints the code being worked on.*

## Milestones

The goal is the shortest path to opening a code file and running a command on it. Estimates assume evenings and weekends.

&#91;embedded content: Jig roadmap · 6 milestones, evenings and weekends\]

By the end of Milestone 4, the full concept works end to end.

### M0: Spike ✅

- Pin `gpui-kit` and get a window running
- Load a file into the editor component
- Confirm the cursor's screen coordinates and the selection range can be read from the component

### M1: Open and edit ✅

- Open a file via CLI argument or file dialog, save with Cmd/Ctrl+S
- Tree-sitter highlighting for two or three languages you use daily
- One file, one window. No tree, tabs or settings.

### M2: Floating command input ✅

- Cmd+K opens a small input anchored below the cursor or selection
- Presets loaded from TOML, filtered as you type, custom text when nothing matches
- Esc closes it and returns focus to the editor

### M3: AI round trip ✅

- `Provider` trait with Anthropic and OpenAI-compatible implementations
- Build the request, parse the JSON reply, show the message in the floating window
- Spinner while waiting, short error on failure

### M4: Apply and review ✅

- Inline diff preview of the change
- Tab or Enter accepts, Esc rejects
- Accepted change applied as one undo step

### M5: Polish ✅

- Hidden titlebar, typography, spacing
- Floating window styling and animations
- Light and dark themes

### After v1: done so far

- OneNord light and dark themes, following the system appearance
- macOS menu bar (Jig, File, Edit) and an app bundle script
- Add commands from inside Jig: Add Command… (⇧⌘K), Cmd+Enter on typed text, Edit Commands File
- Notes on commands: a `comment` setting, and Tab to add a one-off note
- `JIG.md` project rules sent with every command
- Qwen3.8 Flash as the default model
- Project exploration: commands with `explore = true` (or any command with ⌘E in Cmd+K) may call read-only tools (`list_dir`, `read_file`, `search`) inside the project before answering. Ignored and secret-looking files are never visible, there are at most 8 calls, and each step shows live in the waiting bubble.
- Folder and file tree: Open… (and `jig <path>`) accepts a folder as well as a file. A sidebar shows the project, which is the opened folder or the open file's `.git` root. Folders load only when expanded, sort before files, and anything matched by `.gitignore` is dimmed; `.git` and `.DS_Store` are hidden. The open file is revealed and highlighted. ⌘B shows or hides the sidebar (it starts open for a folder, hidden for a single file), ⇧⌘E moves focus between tree and editor, arrows/Enter/Esc navigate, and the edge drags to resize (160–480 px). New File and New Folder buttons in the sidebar header open a name field in the tree, inside the selected folder (or the selected file's folder); `a/b.rs` creates folders on the way, a taken name shows an error, Esc cancels, and a new file opens in a tab. The tree re-reads the disk when the window regains focus and after saving; there's no live file watching yet. GPUI Kit's own tree needs every folder loaded up front, so Jig has its own (`jig-app`'s `file_tree`).
- Tabs: every file opens in its own tab, next to the current one, with its own buffer, unsaved marker and undo history; opening a file that's already open switches to it, and an untouched Untitled tab is reused. The strip sits above the editor and appears only with two or more tabs (as in Safari); same-named files show their folder. ⌘N new file, ⌘W close tab (asks if unsaved; the last tab closed leaves an empty Untitled one), ⇧⌘W close window (asks about every unsaved tab), ⌘1–8 and ⌘9 for the last tab, ⇧⌘] / ⇧⌘[ and ⌃Tab / ⌃⇧Tab to cycle, middle-click to close. `jig a.rs b.rs` opens each file in a tab. Switching tabs keeps a change under review and cancels a command still waiting for the model.

### Next

1. **Cmd+P quick open**, reusing the command input's fuzzy filter.
2. Maybe: a "Show last request" menu item, sending related files automatically, and moving API keys to the Keychain so `Jig.app` works when opened from Finder.

## Command format and AI contract

### Presets

Presets live in a TOML file, so new commands need no recompile. Each has a name, a scope and a prompt.

*Status: the built-ins are in `assets/default-commands.toml`; yours are in `~/.config/jig/commands.toml`, and one with the same name replaces a built-in. Commands can also ask for a note:*

```toml
[[command]]
name = "Create controller"
scope = "cursor"
prompt = "Insert a REST controller at the cursor."
comment = "required"          # none (default) | optional | required
comment_hint = "Entity name"  # placeholder for the note
```

```toml
[[command]]
name = "Add docs"
scope = "selection"   # selection | cursor | file
prompt = "Add concise doc comments to this code."

[[command]]
name = "Create controller"
scope = "cursor"
prompt = "Insert a REST controller for the entity in this file at the cursor."
```

Typing in the floating input filters the presets. Enter runs the highlighted preset. If nothing matches, the typed text runs as a custom command.

### Request

Each request sends the command prompt (or custom text), the file contents, the language, the selection range and the cursor position.

*Status: the region is marked in the file with `<<<SELECTION>>>…<<<END>>>` or `<<<CURSOR>>>`. The request also carries the user's note (`Note: …`) when there is one, and the nearest `JIG.md` (found from the file's folder up to the `.git` root, capped at 16 KB) as project rules. Nothing else is sent, unless the command explores: then the model can read other project files through read-only tools. There is no history between commands.*

### Response

The model must answer with structured output only:

```json
{ "replace": "...new code for the selection or insertion...", "message": "Added docs to 3 methods." }
```

- `message` is capped at about 20 words in the system prompt and truncated by Jig as a safety net.
- A malformed response shows a short error in the floating window and changes nothing.

### Applying a change

- The change shows as an inline diff in place, with the message in a floating bubble beside it.
  *Status: GPUI Kit has no ghost text, so the change is applied provisionally: the new code goes into the buffer, highlighted, while the file is read-only, and the replaced code shows struck through in the bubble. Rejecting undoes just that edit, leaving earlier edits alone.*
- Tab or Enter accepts, Esc rejects.
- An accepted change is one undo step, so Cmd+Z reverts the whole command.
- v1 edits stay inside the current file. Creating new files comes later with a `new_file` action in the schema.

## Out of scope, risks and open questions

### Out of scope for v1

LSP, project search, git, terminal, plugins, settings UI, multi-file edits, and Jig's own buffer and editor element.

*Status: the file tree and tabs are done. Project search exists only as a read-only tool for the model, not in the UI.*

### Risks

- **Cursor coordinates.** *Retired in M0.* The floating UI needs the cursor's screen position from GPUI Kit's editor. If the component doesn't expose it, Jig needs a fork of the component or its own editor element earlier than planned. Checked in Milestone 0.
- **GPUI churn.** APIs move and docs are thin. Pin one version, upgrade deliberately, and use Zed's source as reference.
- **Model output quality.** Local models may break the JSON contract or rewrite more than the selection. Validate responses and reject anything that doesn't parse.
- **Mac look on Linux.** Vibrancy and native titlebar tricks are macOS-only. The clean look has to come from type, spacing and restraint, not platform effects.

### Open questions

- Which languages get tree-sitter support first? *Rust for now.*
- Which local model is the default for testing? *The default is a hosted model, OpenCode Go's Qwen3.8 Flash; Ollama `qwen2.5-coder:7b` is configured for local use.*
- Open source from day one, or later? *Still open.*
- How much should the model explore? *Per command, opt-in, read-only and bounded to 8 tool calls.*
