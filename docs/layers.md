# Layers inside a capability crate

Every capability is a crate (`architecture.toml`'s `[crates]`). Inside one,
the code is laid out in a facade and three layers, each a folder under
`src/`, so what draws, what decides and what keeps state stay apart. Crates
convert one at a time; `jig-planner` was first and is the example to copy.

## The layers

| Layer | Where | Holds | May name | Never names |
| --- | --- | --- | --- | --- |
| facade | `lib.rs`, and small API files beside it | `mod` lines, `pub use`, thin functions that route to a layer | `ui`, `domain`, `store` | IO (`std::fs`, `std::process`, `std::net`, `jig_process`, `trash`); no `impl Render`/`RenderOnce`; at most 150 lines |
| `ui` | `ui.rs` + `ui/` | GPUI views: drawing, events, focus, entities | `domain`, `store` | IO |
| `domain` | `domain.rs` + `domain/` | the business logic, plain Rust | nothing else in the crate | `gpui_kit` and IO |
| `store` | `store.rs` + `store/` | the state and how it's kept | `domain` | `gpui_kit` |

`store` has up to three files, only those the crate needs:

- `session.rs`: in-memory state for this window or process (what's open,
  what's selected, what's been seen).
- `cache.rs`: data derived from elsewhere and rebuilt when stale (an index).
- `persist.rs`: long-term, db-like storage on disk (`.jig/tasks/*.md`).

A file's layer is its first folder under `src/` (`src/ui/form/view.rs` is
`ui`); every file at the top that isn't `ui.rs`, `domain.rs` or `store.rs`
is facade. Files aim for 150 to 400 lines and never pass 500.

### The trait pattern

`domain` never touches the disk. Where it needs something kept or read, it
declares a trait beside the logic that needs it, and `store` implements it:

```rust
// domain/tasks.rs
pub trait TaskStore {
    fn load(&self, id: &str) -> Result<Task>;
    fn save(&self, task: &Task) -> Result<()>;
    // …
}
pub fn toggle_todo(store: &impl TaskStore, id: &str, ix: usize) -> Result<()> { … }

// store/persist.rs
pub struct TaskFolder { project: PathBuf }
impl TaskStore for TaskFolder { … }

// ui/planner.rs
let saved = tasks::toggle_todo(&self.store, id, ix); // self.store: TaskFolder
```

`domain`'s tests implement the trait in memory (`domain/tasks.rs`'s `Tasks`,
`domain/symbols.rs`'s `Files`); `store`'s tests check the real one against a
temporary folder.

## How it's checked

`architecture.toml`'s `[layers]` holds the rules above as data (`uses`,
`forbids`, `forbids_impls`, `max_lines`), and a capability's
`layered = true` turns them on for its crate.
`crates/jig-app/tests/architecture/layers.rs` reads every file of a layered
crate as tokens: the `crate::`, `super::` and `self::` paths say which layer
it reaches, the start of every path says what it names (`use std::{fs, …}`
names `std::fs`), and `impl … for` says what it implements. Test code is
left out. There's no baseline: a crate marked layered must be clean.

```sh
cargo test -p jig-app --test architecture
```

What the check can't see: IO done through another crate's function
(`jig_ai::tasks::save` writes a file, but only `std::fs` is named), and GPUI
reached through another crate (`jig_commands::surface`). Keep those in
`store` and `ui` by hand.

## Converting a crate

1. Read the crate and sort its code: what draws or handles input (`ui`),
   what decides (`domain`), what's kept in memory or on disk (`store`).
2. `git mv` each file into its layer's folder, so history follows it. Add
   `ui.rs`, `domain.rs` and `store.rs` declaring the submodules, each with a
   doc comment saying what the layer holds.
3. Pull logic out of views into `domain` as plain functions on plain data,
   and state bookkeeping into `store/session.rs`. Move code; don't rewrite
   behaviour.
4. Where `domain` read or wrote the disk, declare a trait there and
   implement it in `store/persist.rs` (or `cache.rs`); the view holds the
   store and hands it in.
5. Cut `lib.rs` down to `mod domain; mod store; mod ui;` and the `pub use`
   lines that make the crate's contract, keeping the names `jig-app` uses.
6. Split anything over 400 lines. A view's drawing goes in a child module
   (`ui/form/view.rs`), which sees its parent's private fields; so do
   `testing`-feature helpers (`ui/form/testing.rs`).
7. Set `layered = true` on the crate's capability in `architecture.toml`.
8. Update the crate's paths in `AGENTS.md`, then run `cargo fmt`,
   `cargo clippy --all-targets`, `cargo test` and, if lengths came down,
   `JIG_LENGTH_BLESS=1 cargo test -p jig-app --test file_length`.

## The planner, as it ended up

```
crates/jig-planner/src/
  lib.rs              facade: mod lines and the contract's pub use
  ui.rs               mod lines, the views' re-exports
  ui/planner.rs       TaskPlanner: the list's state and handlers
  ui/planner/view.rs  drawing the list and its cards
  ui/form.rs          TaskForm: the New/Edit Task sheet and its @ list
  ui/form/fields.rs   the form's linkable fields
  ui/form/view.rs     drawing the form
  ui/form/testing.rs  helpers for jig-app's UI tests
  ui/widgets.rs       icons, buttons, fields, checkboxes
  domain.rs
  domain/links.rs     reading and placing links to code
  domain/mention.rs   what @ is typing and what it matches
  domain/symbols.rs   finding declarations; ProjectFiles trait
  domain/tasks.rs     changing and saving tasks; TaskStore trait
  store.rs
  store/session.rs    Board: tasks shown, folded open, seen
  store/persist.rs    TaskFolder (.jig/tasks), ProjectFolder (the walk)
```
