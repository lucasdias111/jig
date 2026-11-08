# What jig-map carries into the Canvas: inventory

Research for [What jig-map can carry into the Canvas](../02-what-jig-map-carries.md). This covers local code only, as of `8c273b6`. Paths are under `crates/jig-map/src/` unless they say otherwise.

## Summary

**For the block catalogue.**
- The Map draws its boxes as **GPUI elements**: absolutely positioned `div`s with text sized to the zoom (`ui/view/draw/cards.rs`, `draw/cards/plan.rs:38`). Only arrows, curves, heads and the grid are painted by hand, in `canvas()` paint passes with `PathBuilder` and `paint_path` (`ui/view/draw/lines.rs:21`, `draw/canvas.rs:30`).
- That is good news for typed blocks. Any GPUI Kit element can go inside a block. That includes `TextView::markdown`, which already renders the agent's replies (`jig-commands/src/ui/chat/transcript.rs:79`) and the hover docs with highlighted code blocks (`jig-app/src/workspace/hover/view.rs:40`). It also includes `StyledText` fed by GPUI Kit's tree-sitter `SyntaxHighlighter::styles` (`gpui-component-0.7.0/src/highlighter/highlighter.rs:1153`).
- Nothing on the Map draws rich text, highlighted code or images today. The code peek is plain monospace lines (`ui/view/input/hover.rs:92`, `draw/overlays.rs:28`).
- The lens's **place grammar and resolver** (`path#Type.method`, `path`, `folder/`, `domain:Name`, `sketch:a/b`, plus suggestions when a name isn't found) is the most valuable piece to reuse. It is how a block gets anchored in code.
- The lens's **drawing kinds** are overlays on a level's boxes. They have no position of their own (`ui/view/lens/landing.rs:33`), so they can't become blocks as they are.

**For layout and motion.**
- The camera (`Camera`, `Camera::showing`, `towards`), the eased `Transition` cross-fade and the per-key `Glide` all work on any keyed rectangles and can be reused (`ui/view/zoom.rs:58-196`, `ui/view/arrangement.rs:95`).
- Labels placed clear of obstacles and kept where they were (`label_spot`, `sticky_spot`) can be reused too.
- Layout comes in two kinds, and both are bound to their level: Plan's hop columns (`domain/layout.rs:156`) and Site's nested sizing, laid out once (`domain/site/sizing.rs`). Neither is a general block packer.
- Site's level-of-detail rule (`Node::opening`, `domain/site.rs:124`) is the model to copy for blocks at a distance.

**GPUI limits.**
- GPUI can't transform a `div`. Only `svg()` takes a transformation (`gpui-pre-0.3.7/src/elements/svg.rs:66`). So every frame of a zoom lays every block's text out again at a new font size.
- The line-layout cache keeps only the previous frame's lines, keyed by font size (`gpui-pre/src/text_system/line_layout.rs:458`).
- The likely result: a smooth zoom over many code blocks shapes the text afresh on every frame, and text under about 5 px is unreadable anyway. This is inferred from the code, not measured.
- Code blocks will need a level-of-detail step (show only the title or a silhouette below some on-screen size) and culling, which Site already does (`ui/view/scene/site.rs:36`).
- Nothing measures GPUI paint time. The only budgets are on the domain side (below).

**For migrating the Map.**
- `MapView` is one struct with one `Level`, one Plan, one camera and one scene (`ui/view.rs:122-196`). That is the main obstacle to "views summoned onto a canvas", where several Plans and a Site might sit side by side.
- The index, outline, Plan and Site builders and the domains are plain `domain/` code with no IO. They can be called once per view and drawn into a block's rectangle.
- Drawings already **belong to a conversation and come back from its transcript** (`AgentEvent::Drew` and `Turn::Drew`, `jig-ai/src/domain/agent.rs:294,313`; `jig-app/src/workspace/map_agent.rs:185,460`). That is exactly "one canvas per conversation". A board can be a snapshot of the replayed calls.
- Export is a second renderer, written by hand as SVG (`ui/view/export/`). Every new block type would need an SVG twin, so export is in the way unless it's rethought.

## 1. Agent-facing tools (`--map-mcp`)

| Item | Where | Verdict |
|---|---|---|
| Tool names, OpenCode and ACP server wiring, permission names, `tool_of` matching each agent's name for a tool | `jig-ai/src/domain/map.rs:28-104` | **As-is.** The names change (`canvas_*`), the mechanism stays. |
| MCP stdio loop: index read in the background, first call waits, domains tools read the project again | `ui/mcp.rs:30-110` | **As-is.** |
| `answer()`: parse, `check` against the index, reply with what was drawn or with suggestions | `ui/mcp.rs:113-141` | **As-is.** It's the right contract for typed blocks: validate, then report back so the agent can fix itself. |
| Tool schemas, written by hand with `json!` | `ui/mcp.rs:143-247` | **Generalise.** One schema per block type, generated from the serde types (or at least kept beside them), not inline JSON. |
| `INSTRUCTIONS`: one long string, written about Map levels | `ui/mcp.rs:24` | **Rewrite.** Keep its rules: read first, name real places, short caption with Markdown links, never touch the user's sketch. |
| All tools marked `readOnlyHint` and allowed without asking | `ui/mcp.rs:162`, `jig-ai/src/domain/map.rs:71` | **As-is.** Drawing never changes code. |
| `map_project` and `map_propose_domains` | `domain/domains/tools.rs`, `domain/domains/tools/` | **As-is**, for the Domain view's actions. |
| Test of the whole exchange | `ui/mcp.rs:253` | **As-is**, as a pattern for block tools. |

## 2. Lens drawings and place resolution

| Item | Where | Verdict |
|---|---|---|
| `Drawing { kind, title, steps, links, ghosts }` and `Kind` (Highlight, Note, Trace, Tour, Group, Propose, Describe) | `domain/lens.rs:23-139` | **Generalise.** Make it one variant of a block enum. Trace maps onto a *code flow* block, Propose onto a *flow change* or *architecture* block, Note onto a caption or annotation. Highlight, Group and Describe stay as *annotations over views*, not blocks. |
| `call(tool, input)`: tool JSON to `Call::Draw` / `Clear` | `domain/lens/calls.rs:7` | **Generalise.** Same shape for each block type. |
| `Place` enum and `resolve`: normalising, `#L12` to the function on that line, `Type.method` under another owner, any real file, folders, domains, sketch boxes, suggestions | `domain/lens.rs:150-215`, `domain/lens/resolving.rs:6` | **As-is.** This is the anchor grammar every grounded block needs. |
| `check`: every place in a drawing resolves | `domain/lens/resolving.rs:227` | **As-is.** |
| `Drawing::places`, `summary` (the transcript row) | `domain/lens.rs:108-139` | **As-is.** |
| `MapView::place` (cached resolve) and `landing` (the box a place lands on at Plan, Site or Domain, as far as boxes are open) | `ui/view/lens/landing.rs:7,33` | **Generalise.** Landing has to go to a *block's* rectangle (a Plan view block, a Site view block, a code block) instead of the one level showing. The fallback order (function, then type, then file, then domain) carries over. |
| `fold`: a trace's steps in a row on one box become one stop with a range | `ui/view/lens.rs:106` | **As-is.** It's needed whenever a flow is drawn over a coarse view. |
| Tour: ←/→, `fly_to`, `fly_to_step`, guide bar | `ui/view/lens/touring.rs:57,75`, `ui/view/lens_draw/tour.rs` | **Generalise.** Flying between blocks instead of levels. A good fit for a "step through" on a code flow block. |
| Lens overlay drawing: outlines, numbered stops, dashed path, words under boxes, edge arrows to boxes out of view, drawings list | `ui/view/lens_draw.rs:151`, `lens_draw/overlay.rs:36,317`, `lens_draw/legend.rs:44` | **Generalise.** Edge arrows and the list carry over. The overlay geometry assumes one scene underneath it. |
| Ghosts with Make Task, Implement, Adopt, Dismiss (`ProposalChoice`) | `ui/view/lens_draw.rs:37-47,271`, `lens_draw/tour.rs:88` | **Generalise.** It's the blocks-to-work path ("You decide"), but tied to the ghost shape. Lift it to any proposed block. |
| Describe: proposed purposes and labels, kept with Tab or Enter | `ui/view/lens/descriptions.rs`, `store/persist/purposes.rs` | **As-is**, as an annotation on Site and Domain views. |
| Pins: a `Drawing` saved by title | `store/persist/pins.rs:13-70` | **Generalise** into boards: a board is many blocks plus their layout, not one drawing. |

## 3. Sketches

| Item | Where | Verdict |
|---|---|---|
| Model: `Sketch { title, level, task, archived, boxes, arrows, notes, frames }`, ids from labels, `ShapeRef` keys | `domain/sketch.rs:33-160` | **Generalise.** Boxes, arrows, notes and frames are the user's half of the catalogue. `level` (`site`, `domain`, `plan:<place>`) has to become "which canvas or board, which view block". |
| Placement relative to the nearest real box (`near` + `at`), arrows to places, notes `on` | `domain/sketch.rs:59-110`, `ui/view/sketch/layout.rs:7-60` | **Generalise.** This is the right way to keep a hand-drawn shape stuck to code as it moves. It needs the landing generalised as in §2. |
| Matching boxes to code (`match_box`, unsure means "?") and arrows (`calls_between`) | `domain/sketch/matching.rs:39,150` | **As-is.** Pure domain code. This is the "is it real yet" signal for feature or architecture blocks. |
| Tools strip, drawing in place, hit tests, undo history, labels | `ui/view/sketch.rs`, `sketch/mouse.rs`, `sketch/hits.rs`, `sketch/history.rs`, `sketch/labels.rs`, `sketch_draw/` | **Generalise.** The interaction carries over. Drawing uses the same scaled-`div` approach. |
| Adopt (ghosts become the user's boxes) | `ui/view/sketch/adopting.rs:67` | **As-is** in idea: the agent proposes, the user takes it. |
| Telling the agent a sketch as data (`sketch_lines`), Make Task, Implement, linking a task back (`link_tasks`) | `ui/view/sketch/telling.rs:77`, `MapEvent::Converse` / `Implement` (`ui/view.rs:114-119`) | **As-is.** The pattern "canvas content serialised as lines for the agent, never code" holds. |
| Persistence `.jig/map/sketches/<title>.toml`, written only when it changed | `store/persist/sketches.rs:13-58` | **As-is**, if sketches stay files. A board may absorb them. |

## 4. Scene and drawing layers

- **Scene model** (`ui/view/scene.rs:20-200`): `Card`, `FrameBox`, `RegionBox` and `Arrow`, each with a key, a rectangle in drawing units, alpha, tint and `Look`. **Generalise.** It's the right idea, a flat list of keyed rectangles and arrows built afresh each frame, but its fields are Map-specific: signature, badges, `hidden: [usize; 3]`, calls. A Canvas scene wants `Block { key, rect, kind, alpha }`, with the kind drawing its own content.
- **`render_scene`** (`ui/view/draw/canvas.rs:215`) stacks regions, frames, arrows behind or in front, cards and labels in one absolutely positioned `div` with opacity. **As-is** in structure.
- **`render_canvas`** (`draw/canvas.rs:5-130`) stacks the old scene fading out, the leaving scene, the current scene, the hover catcher, sketches, the lens, the marquee, the domain overlay, the tool strip, the ruler and the hint. **Generalise.** The layering order is a good template. The level-specific overlays drop out.
- **Text** is either sized to the zoom (`view.scaled(10.5)`, at Plan) or one size on screen (Site, `Look::fixed_text`, `scene.rs:48`). Both ways are needed for blocks: a code block scales, a caption may not.
- **What it can draw today:**
  - *Plain text:* yes.
  - *Rich or Markdown text:* not on the Map. `TextView::markdown` is available and is used elsewhere in Jig.
  - *Highlighted code:* not on the Map. GPUI Kit offers `SyntaxHighlighter` (tree-sitter, the same grammars Jig enables) whose `styles()` gives ranges and `HighlightStyle` for `StyledText`. Jig's editor (`jig-editor`'s `KitEditor`) is a full entity, too heavy to embed by the dozen. A read-only `StyledText` per code block is the practical route.
  - *Images:* none. GPUI has `img()` and `paint_image`.
- **Arrows** (`ui/view/draw/lines.rs`): `Line { from, to, colour, width, dashed, across, pulse, heads }` painted with `PathBuilder`. Bowed or across curves, a moving pulse dot, heads. **As-is** for block connectors.
- **Label placement:** `obstacles` and `label_spot` (`draw/lines.rs:87,142`), `sticky_spot` (`draw/canvas.rs:137`) with leader lines back to the arrow. **As-is.** This is the "labels in the gaps, never jumping" behaviour the Canvas wants.
- **Colours:** `look_colour` and `theme::canvas` (`ui/view/draw.rs:76,138`), and the agent accent `jig_commands::surface::agent_accent`. **As-is.**

## 5. Animation and transitions

- **One animation clock:** `Render` asks for `request_animation_frame` while a transition, glide, firing pulse or fade is under way (`ui/view.rs:369-381`). No timers. The frame is computed from `Instant`. **As-is.**
- **`Camera`** (`zoom.rs:58-101`): `showing(rect, screen)` and `towards` (straight across, zoom by ratio). **As-is.**
- **`Transition`** (`zoom.rs:103-196`, `frame` at `:257`): the old scene with its own camera fades out while the new one flies in, eased cubic, 0.45 s (`FLIGHT`), 0.3 s pans. **Generalise.** Today it swaps whole levels. On a canvas it becomes "summon a view": a block growing out of another, or out of an anchor.
- **Glide** (`arrangement.rs:95`, `GLIDE = 0.22` at `view.rs:65`), boxes leaving fading out (`FADE_OUT`), boxes arriving fading in (`live.rs:35`). Keyed by box key, so any re-layout of blocks can glide for free. **As-is**, and it's the core of "blocks move, don't jump".
- **Level thresholds and gesture locks** (`PLAN_OUT`, `SITE_IN`, `DOMAIN_TO_SITE`, `zoom_lock`; `zoom.rs:25-55`, `zoom/stepping.rs`, `zoom/cameras.rs`). **In the way** on an ad-hoc canvas. Keep them inside a Domain/Site view block, if views keep their zoom-through.
- **Ruler** (`ui/view/ruler.rs`). **In the way.** It's tied to the four levels.
- **Live and debug glow** (`domain/debug.rs`, `ui/view/live.rs`, `Firing`): logpoints make functions glow and calls pulse. **As-is** for a code-flow block grounded in real functions: an animated trace for free.

## 6. Layout

- **Plan** (`domain/layout.rs:156`): columns by hop, frames per owner, `MAX_COLUMN` wrapping. Hand-dragged places are kept (`Placed::put`, `refit`). **Reuse inside a Plan view block.** It's not a canvas packer.
- **Site** (`domain/site/building.rs:7`, `domain/site/sizing.rs`): nested boxes sized by their contents, laid out once. **Reuse inside a Site or Domain view block.**
- **Domain regions and link labels** (`domain/domains/level.rs`, `ui/view/domain_draw/`). **Reuse inside a Domain view block.**
- **`Rect`** (`domain/layout.rs:22-70`: `overlaps`, `union`, `around`). **As-is.**
- **Missing:** a general placer for blocks of mixed sizes that keeps what the user moved, avoids overlap and places a new block beside what it's about. Sketch placement by `near`/`at` is the nearest thing. This gap belongs to [Layout and motion](../07-layout-and-motion.md).

## 7. Export

- `export_svg` turns the scene into SVG strings (`ui/view/export.rs:106`, `export/level.rs:8`, `export/lens.rs`, `export/sketches.rs`). resvg turns that into a PNG at 2× with system fonts (`export/svg.rs:6`). The longest side is 3200 (`export.rs:39`).
- **In the way as built.** It's a second renderer that copies every visual decision by hand. Highlighted code or Markdown in SVG would mean writing a text layout engine.
- Options for the spec:
  - Grab the GPUI frame. Not offered by GPUI Kit 0.7, so this would need checking.
  - Export only the outline of a board (Copy as Markdown, `domain/domains/text.rs`).
  - Keep SVG for shapes and arrows and drop rich content to plain text.

## 8. Persistence under `.jig/map/`

| File | Code | Verdict |
|---|---|---|
| `domains.toml` (committed) | `store/persist/domains.rs`, `domain/domains.rs` | **As-is.** Project knowledge, not canvas state. |
| `sketches/<title>.toml` (committed) | `store/persist/sketches.rs` | **As-is or folded into boards.** |
| `pins/<title>.toml` (committed) | `store/persist/pins.rs` | **Replace with boards.** |
| `purposes.toml` (committed) | `store/persist/purposes.rs` | **As-is.** |
| `places.json` (kept out of git) | `store/persist/places.rs` (`keep_out_of_git`, `:27`) | **Generalise** to per-board layout and viewport. |
| Agent drawings | not on disk; replayed from the conversation's `Turn::Drew` (`jig-app/src/workspace/map_agent.rs:460`) | **As-is.** It already *is* one canvas per conversation. |

Everything is keyed by place names, never screen positions, except `at` offsets and `places.json`. That's why committed files survive edits and merges. Keep that rule for boards.

## 9. Wiring in jig-app

- `workspace/map.rs` (325 lines) holds the one `MapView` per workspace, ⌘B, pinch-out, Go Back places and server calls.
- `workspace/map_agent.rs` (496 lines) handles conversations started from the Map, `agent_drew` (`:185`), `sync_map_drawings` (`:234`), rows in the transcript, proposals and links.
- `MapEvent` (`ui/view.rs:93-120`) is the whole contract outward.
- **Generalise.** The event set mostly carries over. `MapView::question` (`ui/view/ask.rs:31`), which gives the agent what's selected plus an outline, never code, is the Canvas's "agent reads the canvas".

## 10. Performance budgets and tests

- `big_project_live` (`domain/site/tests.rs:187`, ignored, `JIG_MAP_PROJECT`, release budgets, 8× in debug):
  - read in 20 s
  - read again in 4 s
  - grouping and Site in 4 s
  - **a Site frame's scene in 30 ms** at four zooms
  - a Plan in 30 ms
- These are **domain-side only**. Nothing times GPUI layout or paint, and 30 ms is already over a 16 ms frame.
- Site caps arrows at 160 (`ui/view/scene.rs:23`) and culls boxes outside the view (`scene/site.rs:36`). Site's arrows are cached while the shown set stays the same (`site_arrows`, `view.rs:160`).
- The pictures harness `map_pictures_live` (`jig-app/src/workspace/tests/map/pictures.rs:77`) draws through export, not GPUI. The same goes for `sketch_pictures_live` and `map_agent_live`. It's useful for checking a design without a screen, but only as good as the SVG twin.
- There are about 74 unit tests in `jig-map`, plus UI tests in `jig-app/src/workspace/tests/map*` and `map/tests.rs`.
- **For the Canvas:** add a GPUI frame-time test (N code blocks, a zoom sweep, measured around `draw`) before committing to code-heavy blocks. Plan for level-of-detail and culling from day one.

## What fights the Canvas, in one list

1. One `Level` and one camera per `MapView` (`ui/view.rs:122-196`). Views need to become blocks, each with its own builder output and its own inner camera or level-of-detail state.
2. Lens drawings with no position of their own: they land only on the level showing (`landing.rs:33`).
3. Export as a hand-written second renderer (`ui/view/export/`).
4. Thresholds between levels, the ruler and gesture locks: global today, at most per view block tomorrow.
5. Schemas and instructions written as inline strings about levels (`ui/mcp.rs:24,143`).
6. No rich text or code rendering yet, and no frame-time guard for GPUI.
