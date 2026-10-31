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
4. **The status bar's message** (`status_note`), shown by formatting and
   the language servers. `pending_save` stays: it only holds the task that
   finishes the workspace's own save, and has nothing to draw or decide.
5. **Go Back and Go Forward** (`history`). Nothing to draw, so not an
   entity: `History` takes over its own stacks (`remember`, `step`), and
   the workspace keeps working out where "here" is and going there.
6. **Breakpoints** (`breakpoints`) and **live points** (`live`): plain
   structs that keep their own rules. Then **run configurations and the run
   picker**: an entity, since it loads in the background and has a picker
   to draw.
7. **The run session, its output panel and the debugger** (`runs`, with
   `run.rs` and `debug.rs` together): one `RunSession` holds the program,
   its output and the `DebugSession`, and its poll loop takes in the
   debugger's messages, so they move as one. The output panel (lines,
   selection, scroll, height, focus) is the natural entity; the debugger's
   pauses still reach the tabs (the paused line), the Map and the hover
   through the workspace, so they come out as events.
8. **Git**: the branch picker and the change popup, as entities. The
   change bars, the Git view's panel and the repository watch stay: the
   bars are per tab and redrawn on every edit, the panel is the sidebar's
   and watched only while it shows. The **sidebar** stays too: its width,
   motion and which view shows are layout, which is the workspace's, and
   the file tree is already an entity.
9. **Find in Files** and **Go to File** (`find_in_files`, `last_find`,
   `quick_open`, `file_index`, `recent_files`): these are modals, so this is
   when `modal_open` becomes a question asked of each modal entity instead
   of a list of fields.
10. **Planner** (`planner`), then **the Map** (`map`, `pinched`) and the
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
- **Find and Replace in the file** lives in `workspace/find_panel.rs` as
  `FindInFile`: the panel, its last query, and the editor it searches, so
  closing drops the highlights there without the workspace walking every
  tab. It emits `FindDismissed` when closed from inside; the Esc chain,
  switching tabs and modes, and the palette close it through
  `Workspace::close_find_panel`, as before. The workspace places it at the
  top right of the code.
- **The status bar's message** lives in `workspace/status_bar.rs` as
  `StatusMessage`: it shows a message, clears it after a while and draws
  it. `Workspace::show_status` asks it to show one, and the workspace
  observes it, since whether there's a message decides what the status bar
  holds. `pending_save` stayed on the workspace, as above.

- **Go Back and Go Forward**: `History` in `workspace/navigation.rs` now
  keeps its own stacks behind `remember` and `step` (skipping deleted
  files, capping its length), with unit tests that need no window. The
  workspace works out where it is and goes to the place `step` gives.

- **Breakpoints and live points**: `Breakpoints::all` hands out every
  file's lines, and `LivePoints` works out which files changed (`listen`)
  and lights the Map up (`fire`), so the workspace stops reaching into
  either. Plain structs, with a unit test for `listen`.
- **Run configurations and the run picker** live in
  `workspace/run/configurations.rs` as `RunConfigurations`: the
  configurations last loaded and for which project, the one ⌃R runs, the
  background load and the picker. It emits `Chosen { config, debug }`,
  `Edit`, `Closed { refocus }` and `LoadFailed`; the workspace opens the
  picker (closing the other modals first, as before), runs or debugs what
  was chosen, places the picker, and observes it for the title bar's name.
  Opening another project resets it, as it reset `RunState` before.

- **The run output** lives in `workspace/run/output.rs` as `RunOutput`:
  the lines, their selection and scroll, the output's focus, drawing the
  list, mouse selection, ⌘C, ⌘A and the right-click menu, trimming past
  50,000 lines. One for the window, started afresh by each run, so the
  output keeps its focus across runs as before; a clicked `file:line` link
  comes out as `OpenLink`. Run and the debugger add lines through it. The
  session itself (`RunSession`: the process, how it ended, the
  `DebugSession`) stays on `RunState`, and the panel's height and drag stay
  with the workspace's layout. The debugger's state machine stays too:
  each adapter message it handles reaches the tabs (the paused line), the
  Map, breakpoints or the hover, so as an entity it would only send all of
  that back as events. Its call stack and variables view could become an
  entity later, if the debugger grows.

- **Git's branch picker** lives in `workspace/git/branches.rs` as
  `BranchSwitcher`: it opens the picker, reads the branches in the
  background and emits `Switch`, `Create` or `Closed { refocus }`; the
  workspace saves files and checks out, as before.
- **Git's change popup** lives in `workspace/git/hunk.rs` as `HunkPopup`:
  the change, its lines as they are now (any edit to the tab closes it, so
  they're taken when it opens), where it shows, and its drawing; Revert
  comes out as `RevertHunk`, which the workspace makes as one undo step. It
  shows only while its tab is open, as before.

- **Find in Files** lives in `workspace/find.rs` as `FindInFilesPanel`:
  the panel and the last search (query and options), its drawing, and the
  events `Open { path, range }`, `Indexed`, `Dismissed` and `Blurred`. The
  workspace picks the query (the selection, else the last search), goes to
  the match and remembers the place for Go Back; Go to References fills
  the panel through `panel()`.
- **Go to File** lives in `workspace/go_to_file.rs` as `GoToFilePanel`: the
  panel and the files shown in this window, most recent first, which it
  offers before the rest. Opening another project resets it, as before.
  The project's last walk (`FileIndex`) stays on the workspace: both
  panels and Go to Definition's guesses share it, so each panel is handed
  it as it opens and reports a new walk back.
- **`modal_open` asks each modal** (`modal_open(cx)`): the palette and the
  planner's form are still fields, the two panels answer for themselves.
  Esc's fallback, for when the keyboard went back to the code under one,
  closes them all through `close_modals`; opening one closes the others
  through each one's `close`, as before.

## Not every field becomes an entity

An entity earns its place when a feature has state of its own and draws or
decides something with it. A field that only holds a task driving the
workspace's own work (`pending_save`), or a counter, stays where it is:
wrapping it would add a hop without taking anything off `Workspace`. Plain
state with rules of its own (`History`) becomes a struct with methods, so
the workspace stops reaching into it.
