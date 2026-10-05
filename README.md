# Jig

<img width="1192" height="872" alt="image" src="https://github.com/user-attachments/assets/971d677d-1199-4ab0-bb61-817a5e34097f" />

<img width="1192" height="872" alt="image" src="https://github.com/user-attachments/assets/3c88a572-f2ee-449c-85d3-63776015f029" />


A macOS-first code editor where AI works through jigs, small commands at
the cursor, not through a chat panel.

Select some code, press **⌘K**, pick a jig or type a prompt, and the
change appears inline. **Tab** accepts it, **Esc** rejects it. Nothing
reaches the disk until you accept it.

The name comes from woodworking: a jig guides precise, repeatable cuts made by
hand, and that's what each jig is for your code. IDE refactorings are precise
but fixed, and agents are open-ended but take you away from the code. Jig
keeps the refactoring interaction and puts AI behind it.

## Two lanes

The ⌘K input has a **Quick | Agent** switch. Tab flips it.

- **Quick** makes one model call that rewrites only the selection, the
  cursor position or the file. It sees the current file and the nearest
  `AGENTS.md`, nothing else, and takes 2-5 s.
- **Agent** hands the task to [OpenCode](https://opencode.ai), which can read
  and edit the whole project. Each edit opens as a preview in its file's tab
  before it's written, and you accept or reject it like a quick edit. Takes
  15-60 s.

Replies are one sentence of at most 20 words, or nothing.

Jig also has tabs, a file tree, Go to File (⌘P), Find in Files (⇧⌘F),
⌘-click go to definition and autocomplete through any installed language
server, and light and dark themes that follow the system.

## Building

Jig is developed on macOS and also builds on Linux and Windows. You need
Rust; `rust-toolchain.toml` pins the toolchain, so
`rustup` installs the right version on the first build.

```sh
cargo run -p jig-app -- path/to/file-or-folder
```

On macOS, to build an app bundle at `target/Jig.app`:

```sh
script/bundle-macos.sh
```

## Setup

Jig reads API keys from environment variables only, never from files. The
default provider is OpenCode Go:

```sh
export OPENCODE_API_KEY=...
```

On macOS, an app opened from Finder doesn't see your shell environment, so
start it from a terminal:

```sh
target/Jig.app/Contents/MacOS/Jig path/to/project
```

The Agent lane also needs the `opencode` CLI on your `PATH`.

### Configuration

Everything lives in `~/.config/jig/` (or `$XDG_CONFIG_HOME/jig/`):

- `config.toml`: model providers and which one to use. Jig writes a starter
  file with OpenCode Go, Anthropic and Ollama entries.
- `jigs.toml`: your own jigs. A jig with the same name as a built-in one
  replaces it.
- `settings.toml`: editor settings, also editable in the Settings window.

A provider looks like this:

```toml
default = "claude"
agent_model = "anthropic/claude-sonnet-5-5"  # optional, OpenCode's naming

[[provider]]
name = "claude"
kind = "anthropic"            # or "openai" for any OpenAI-compatible server
base_url = "https://api.anthropic.com/v1"
model = "claude-sonnet-5-5"
api_key_env = "ANTHROPIC_API_KEY"
```

A jig looks like this:

```toml
[[jig]]
name = "Add tests"
scope = "selection"           # "selection", "cursor" or "file"
prompt = "Write unit tests for this code."
comment = "optional"          # ask for a note, e.g. "edge cases only"
```

See [`assets/default-jigs.toml`](assets/default-jigs.toml) for the built-in
jigs.

### Project rules

Put an `AGENTS.md` in your project and Jig sends it with every quick run.
OpenCode reads it for agent runs. Use it for conventions the model should
follow.

## Development

```sh
cargo test                     # includes headless UI tests
cargo clippy --all-targets     # kept at zero warnings
cargo fmt
```

[`AGENTS.md`](AGENTS.md) describes the architecture, the crate layout and the
prompt contract.

`vendor/gpui-base` is a patched copy of `gpui-base` 0.7.0 that fixes mouse
handling in scrolled editors. It will be removed once an upstream release
includes the fix.

## License

[Apache-2.0](LICENSE). Language icons are from
[Simple Icons](https://simpleicons.org) (CC0-1.0); see
[`assets/icons/README.md`](assets/icons/README.md).
