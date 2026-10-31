# Phase 4: thinning `Workspace`

Phases 1 to 3 put every capability in a crate of its own, leaving
`crates/jig-app` as the shell. What's left of the god object is `Workspace`
itself: `workspace.rs` and 38 files under `workspace/` that each add an
`impl Workspace` block, reaching each other through its fields. This phase
gives each feature its own state in a GPUI entity, so `Workspace` keeps only
what is truly shared.

## Where it stands

`Workspace` has 38 fields. Counting which `workspace/` files read or write
each one (`self.x` or `this.x`):

| Field | Files | Who |
| --- | --- | --- |
| `home` | 26 | nearly everything (is a file open?) |
| `tabs` | 24 | nearly everything, mostly through `editor()`, `tab()`, `document()` |
| `active` | 12 | tabs, agent, format, git, run, debug, snippets, session, … |
| `settings` | 12 | lsp, format, breakpoints, brackets, code menu, map, … |
| `palette` | 10 | commands, agent, mode, planner, find, go to file, git, run, tabs |
| `mode` | 9 | mode, agent, commands, home, map, navigation, planner, tabs |
| `tree` | 7 | sidebar, git, go to file, mode, planner, tabs |
| `find_in_files`, `quick_open` | 7, 6 | modal bookkeeping in commands, git, run, definitions |
| `map` | 5 | map, map agent, agent, mode, planner |
| `home_focus` | 5 | home, agent, find, go to file |
| `chat`, `next_run_id` | 4, 4 | agent, commands, map agent, format |
| `note` | 3 | commands, tabs, selection button |
| `runs`, `planner`, `live` | 3, 3, 2 | run and debug; planner, mode, commands; debug and live |
| `git`, `sidebar`, `file_index`, `recent_files`, `status_note` | 2 each | their own file and one neighbour |
| `go_to_line`, `rename`, `find_panel`, `last_find_query`, `last_find`, `history`, `breakpoints`, `pinched`, `pending_save`, `file_watch`, `config_path`, `model_defaults`, `palette_openings`, `selection_button`, `tab_scroll` | 1 | only their own file |

Methods tell the same story from the other side. Counting which files call a
`Workspace` method defined in another file: `tabs.rs` is called from 34 files,
`commands.rs` (notes, the floating anchor, `modal_open`, the review keys)
from 20, `home.rs`, `mode.rs`, `map.rs` from 12, `agent.rs`, `go_to_file.rs`
(`project_root`) from 11. At the other end, `go_to_line.rs`, `status_bar.rs`,
`code_menu.rs`, `preferences.rs`, `quick_fix.rs`, `occurrences.rs`,
`brackets.rs` and `completions.rs` are each called from one place: the
render or the key routing.

So there are two kinds of state. **Shared**: the tabs and the editor on
screen, `home`, the mode, the settings, focus, the modal bookkeeping
(palette, quick open, find in files, the planner's form) and the note. And
**owned**: everything else, which one feature file reads and writes, plus a
handful of calls from the render, the Esc chain or `replace_project`.

## Target shape

A thin `Workspace` that owns:

- the tabs and the editor on screen (`tabs`, `active`, `tab_scroll`,
  `home`, `home_focus`), and the accessors everyone uses (`editor()`,
  `tab()`, `document()`, `project_root()`);
- the mode and the layout (sidebar width and motion, docked conversation,
  run panel height);
- focus and key routing: `focus_main`, `focus_editor`, the Esc, Enter and
  Tab capture handlers, and `modal_open` / `keys_elsewhere`;
- the shared floating UI: the palette, the note, `floating_anchor`;
- the settings as last applied, and fanning a change out.

Everything else is a feature entity, `Entity<Feature>`, held in a field and
created in `Workspace::new`. Each feature:

- owns its state and draws itself (`impl Render`), so the workspace's render
  only places it, e.g. `.when(open, |this| this.child(self.feature.clone()))`;
- says what happened by emitting events (`EventEmitter<FeatureEvent>`), which
  `Workspace::new` subscribes to with `cx.subscribe_in`, so the workspace does
  what crosses features (focus, notes, opening files, Go Back history);
- is asked things through narrow methods (`is_open`, `close`), never by the
  workspace reaching into its fields;
- gets what it needs from the workspace as arguments when it's asked to do
  something (an anchor, an `EditorHandle`, a project root), not a back
  reference, unless it renders on its own schedule (the selection button
  already holds a `WeakEntity<Workspace>` for that, and is the precedent).

Features that are the window's moods rather than its parts (the agent
conversation, the Map, the Planner) come last and may keep some state on the
workspace: what they change is the mode, focus and the Esc chain, which stay
there by design.

## Order

Easiest and most self-contained first, so the pattern settles on features
where a mistake is cheap and obvious:

1. **Go to Line** (`go_to_line`). One field, one file, no one else reads it.
   Done first; see below.
2. **Rename Symbol's field** (`rename`). One field, one file; the edits it
   makes go through the workspace's tabs, so it emits the new name and the
   workspace renames.
3. **Find and Replace in the file** (`find_panel`, `last_find_query`). Two
   fields, one file; closed from the Esc chain, `tabs.rs` and `mode.rs`
   through `close`.
4. **Status note and pending save** (`status_note`, `pending_save`):
   format-on-save's state, read by the status bar.
5. **Go Back and Go Forward** (`history`). One file owns it; ten call
   `remember_location`, which becomes a method on the entity taking `here`.
6. **Breakpoints** (`breakpoints`) and **live points** (`live`), then
   **Run and Debug** (`runs`, `next_run_id` stays shared): big, but their state
   already sits in `RunState`, `Breakpoints` and `LivePoints`, so it's mostly
   moving those behind entities and turning `debug.rs`'s calls into events.
7. **Git** (`git`, with `workspace/git/watch.rs`) and the **sidebar**
   (`sidebar`, `tree`): the gutter bars need the tabs, so the entity is told
   about tab changes.
8. **Find in Files** and **Go to File** (`find_in_files`, `last_find`,
   `quick_open`, `file_index`, `recent_files`): these are modals, so this is
   when `modal_open` becomes a question asked of each modal entity instead
   of a list of fields.
9. **Planner** (`planner`), then **the Map** (`map`, `pinched`) and the
   **Map's agent**, then **the agent** (`agent`, `chat`), the largest
   (3,300 lines) and the one most tangled with the Esc chain and review.

Each step is one commit: behaviour kept, the UI tests passing unchanged except
for paths (`workspace.read(cx).x.is_some()` becoming
`workspace.read(cx).x.read(cx).is_open()`).

## Risks

- **Focus.** A feature that used to call `self.focus_main(window, cx)` now
  emits an event the workspace handles a moment later, in the same effect
  flush. The order of focus changes must stay as it was: e.g. Go to Line
  refocuses the code before jumping, so a jump from the Planner lands in
  Code mode. Subscriptions need `subscribe_in` (with the window) for that.
- **Key routing.** Actions are dispatched on the workspace's root element
  under the `Workspace` context; feature entities rendered as children are
  inside that context, so a key bound there still reaches the workspace. A
  feature that starts handling its own actions must not change which
  element is focused when the key arrives, or bindings in `CodeEditor` and
  the feature crates' contexts stop matching.
- **Esc.** `on_escape` is a capture handler that tries, in order: a Map
  field, a modal whose focus went back to the code, `keys_elsewhere`, the
  review, the hunk popup, the find panel, the hover, a problem, the note, a
  completion list or snippet, the agent. Moving a feature out must keep its
  place in that order, so the chain asks the entity (`close`) rather than
  the entity listening for Esc itself.
- **Draw order.** The floating pieces are `deferred` children of the root in
  a fixed order (palette, task form, quick open, find in files, rename, go to
  line, run picker, branch picker, hunk popup, hovers, selection button,
  conversation, note), and that order decides what draws on top. A feature
  entity is placed where its render call was, and adds no element while
  closed, so the layout tree is unchanged.
- **Re-entrancy.** An entity's event is handled after its update returns, so
  a handler may freely update the entity again, but code that used to read
  the field right after setting it now sees the change only after the
  update. Keep reads of feature state out of the middle of a feature's own
  update.
- **Repaint.** The workspace used to `cx.notify()` itself when a feature
  opened or closed. Features notify themselves; the workspace still notifies
  when its own render depends on the change (e.g. whether to place the
  feature).

## Done so far

- **Go to Line** lives in `workspace/go_to_line.rs` as `GoToLineField`: it
  owns the field and its place, draws itself, and emits `Go(target)` or
  `Closed { refocus }`. The workspace opens it (`open(anchor, …)`), places it
  while open, and on its events refocuses, jumps and remembers the place for
  Go Back, as before.
- **Rename Symbol's field** lives in `workspace/rename.rs` as `RenameField`:
  it keeps the tab and range of the name it opened on, and emits
  `Rename { name, tab, range }` or `Closed { refocus }`. Renaming (the
  server's edits, or this file's whole-word uses) stays on the workspace,
  since it edits tabs.
