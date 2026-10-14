<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="assets/app-icon/jig-dark.png">
    <img src="assets/app-icon/jig.png" width="128" alt="Jig logo">
  </picture>
</p>

# Jig

<img width="1123" height="798" alt="Jig editing a file" src="https://github.com/user-attachments/assets/3359dfad-a506-4c8e-8a56-5474d16d8a00" />

Since I started bringing AI into my workflow I've noticed a trend: every agentic
workflow seemed to pull me away from the code, even though I, as the engineer,
am the one who's supposed to own it. That didn't feel right. I didn't see why I
had to open a second program, or move to the terminal, to use AI on the code I
was already looking at.

So I went looking for an editor that made AI part of my workflow with me as the
captain, so I stay in control and keep ownership of the code. Sometimes I want
AI as a second opinion, sometimes to write a bit of boilerplate, and sometimes
as an agent implementing a whole feature. But I want to decide when to do which,
and I don't want to wrestle an agent that insists on writing bloated code for
nothing.

Lately I've heard the AI revolution compared to autopilot becoming common in
planes, and I actually like the analogy. My addition to it: today's AI tools
are like putting the pilot in a passenger seat and expecting them to fly the
plane through a microphone. The plane might fly and land safely. Chances are it
won't, and if something goes wrong, it sure won't go well. A pilot still holds
the controls, still turns the knobs, and knows when to use which tool to fly
safely. I want the same thing for software engineers in the age of AI, and I
don't want to give in to cognitive decline because I've stopped looking at the
code.

I couldn't find that editor, so I built Jig.

## The idea

**You keep your hands on the controls.** IDE refactorings are precise but fixed.
Agents are open-ended but take you away from the code. Jig takes the refactoring
interaction and puts AI behind it.

The name comes from woodworking: a jig is a guide you make so you can do the
same precise cut by hand, again and again. In Jig, each saved AI command is one.
"Simplify", "Add docs", "Fix this error", or whatever you find yourself asking
for every day.

A few things I hold it to:

- **Near the code.** Everything happens at the cursor. No side panels, unless you
  pin one there yourself.
- **You decide.** Every change shows up as a preview and needs an explicit
  accept. Nothing reaches the disk without you looking at it.
- **Brief.** A quick jig answers in one short sentence, or not at all.
- **Keyboard first.** Few shortcuts, each one obvious.
- **Out of the way.** The code is the interface.

## What it does

Select some code, press **⌘K**, pick a jig or type what you want, and the change
shows up inline. **Tab** keeps it, **Esc** throws it away.

<img width="1134" height="813" alt="Jig previewing a change" src="https://github.com/user-attachments/assets/ceabdaed-93f9-4953-a564-bb4d00b9d1e5" />

- **Two modes, one switch.** *Jig* mode makes one quick, focused edit to the
  selection, line or file, in a few seconds. *Agent* mode hands a bigger job to a
  coding agent (OpenCode, Claude Code, Codex, Gemini CLI, or any agent that speaks
  the Agent Client Protocol) in a small conversation next to the code. Every edit
  it wants to make still opens as a preview you accept or reject, and shell
  commands and web access wait for your go-ahead.
- **Your own jigs.** Save any prompt as a jig and run it again with a couple of
  keystrokes. Add a note on the fly (`docs: terse, no examples`).
- **Any model.** Anthropic, OpenAI-compatible providers, or a model running on
  your own machine. Jig doesn't pick one for you.
- **Tasks.** A small planner in the sidebar. Agents can save their plans there,
  and you can hand single steps back to Jig or the agent.
- **A proper editor around it.** Language servers (go to definition, completion,
  rename, diagnostics, hover, formatting), Git gutter and panel, run
  configurations, a debugger over DAP, find in files, and themes that follow
  your system's light and dark.

It's macOS-first, and also builds on Linux and Windows.

<img width="597" height="411" alt="Jig's command palette" src="https://github.com/user-attachments/assets/1b28e39a-5494-4138-8174-a2a59ec864ab" />

## Getting it

### Download

[Download Jig for macOS](https://github.com/lucasdias111/jig/releases/latest/download/Jig-macos-arm64.zip)
(Apple Silicon). Unzip it and move `Jig.app` to Applications. The app isn't
signed yet, so macOS will block it the first time; this lets it open:

```sh
xattr -dr com.apple.quarantine /Applications/Jig.app
```

Linux and Windows builds are on the
[releases page](https://github.com/lucasdias111/jig/releases/latest).

Then connect a model provider in Settings > Providers. For Agent mode, pick an
agent in Settings > Agent and install it from there if you don't have it.

### Build and run

You need Rust; `rust-toolchain.toml` pins the version.

```sh
cargo run -p jig-app -- path/to/project
```

On macOS, `script/bundle-macos.sh` builds `target/Jig.app`. On Linux, GPUI needs
a few system libraries first (see `.github/workflows/ci.yml` for the list).

### Contributing

Issues and pull requests are welcome. Before opening one:

```sh
cargo test
cargo clippy --all-targets
cargo fmt
```

Tests run headless, UI included, and clippy is kept at zero warnings.
[`AGENTS.md`](AGENTS.md) explains how Jig works and how the code is laid out;
it's the best place to start. If you're changing how a feature works, update it
too.

## License

[Apache-2.0](LICENSE). Language icons are from
[Simple Icons](https://simpleicons.org) (CC0-1.0); see
[`assets/icons/README.md`](assets/icons/README.md).
