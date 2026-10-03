# Jig: Plan

Oct 3, 2026 · @Lucas

Jig is a code editor built to give developers control back. AI does the work through small, contained commands run right at the cursor, instead of in an agent that lives somewhere else.

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

The commands are what Jig is testing. The text editor is the means. For v1, the editing surface is GPUI Kit's code editor component, which already handles tree-sitter highlighting, selection and multi-cursor. All effort goes into the command layer.

The command layer only talks to the editor through a small interface: buffer text, selection range, cursor position, cursor screen coordinates, and "apply this edit as one undo step". That keeps the door open to replacing the component with Jig's own editor element later, without touching any AI code.

### GPUI distribution

The official gpui crate on crates.io lags far behind upstream, and GPUI distributions cannot be mixed in one dependency graph. Jig depends on `gpui-kit` and nothing else GPUI-related, pinned to an exact version.

### Crate layout

- `jig-app`: window, workspace, keybindings, theming
- `jig-editor`: the editor adapter (wraps GPUI Kit's component behind the interface above)
- `jig-commands`: preset loading, filtering, the floating input and reply UI, diff preview
- `jig-ai`: the `Provider` trait, prompt building, response parsing

### AI providers

One `Provider` trait, two implementations at first:

- Anthropic API
- OpenAI-compatible endpoint, which covers Ollama and llama.cpp for local models

Requests run off the main thread. The UI shows a small spinner in the floating window until the result is ready.

## Milestones

The goal is the shortest path to opening a code file and running a command on it. Estimates assume evenings and weekends.

&#91;embedded content: Jig roadmap · 6 milestones, evenings and weekends\]

By the end of Milestone 4, the full concept works end to end.

### M0: Spike

- Pin `gpui-kit` and get a window running
- Load a file into the editor component
- Confirm the cursor's screen coordinates and the selection range can be read from the component

### M1: Open and edit

- Open a file via CLI argument or file dialog, save with Cmd/Ctrl+S
- Tree-sitter highlighting for two or three languages you use daily
- One file, one window. No tree, tabs or settings.

### M2: Floating command input

- Cmd+K opens a small input anchored below the cursor or selection
- Presets loaded from TOML, filtered as you type, custom text when nothing matches
- Esc closes it and returns focus to the editor

### M3: AI round trip

- `Provider` trait with Anthropic and OpenAI-compatible implementations
- Build the request, parse the JSON reply, show the message in the floating window
- Spinner while waiting, short error on failure

### M4: Apply and review

- Inline diff preview of the change
- Tab or Enter accepts, Esc rejects
- Accepted change applied as one undo step

### M5: Polish

- Hidden titlebar, typography, spacing
- Floating window styling and animations
- Light and dark themes

## Command format and AI contract

### Presets

Presets live in a TOML file, so new commands need no recompile. Each has a name, a scope and a prompt.

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

### Response

The model must answer with structured output only:

```json
{ "replace": "...new code for the selection or insertion...", "message": "Added docs to 3 methods." }
```

- `message` is capped at about 20 words in the system prompt and truncated by Jig as a safety net.
- A malformed response shows a short error in the floating window and changes nothing.

### Applying a change

- The change shows as an inline diff in place, with the message in a floating bubble beside it.
- Tab or Enter accepts, Esc rejects.
- An accepted change is one undo step, so Cmd+Z reverts the whole command.
- v1 edits stay inside the current file. Creating new files comes later with a `new_file` action in the schema.

## Out of scope, risks and open questions

### Out of scope for v1

LSP, file tree, tabs, project search, git, terminal, plugins, settings UI, multi-file edits, and Jig's own buffer and editor element.

### Risks

- **Cursor coordinates.** The floating UI needs the cursor's screen position from GPUI Kit's editor. If the component doesn't expose it, Jig needs a fork of the component or its own editor element earlier than planned. Checked in Milestone 0.
- **GPUI churn.** APIs move and docs are thin. Pin one version, upgrade deliberately, and use Zed's source as reference.
- **Model output quality.** Local models may break the JSON contract or rewrite more than the selection. Validate responses and reject anything that doesn't parse.
- **Mac look on Linux.** Vibrancy and native titlebar tricks are macOS-only. The clean look has to come from type, spacing and restraint, not platform effects.

### Open questions

- Which languages get tree-sitter support first?
- Which local model is the default for testing?
- Open source from day one, or later?
