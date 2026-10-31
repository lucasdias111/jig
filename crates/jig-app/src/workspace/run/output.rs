//! The run panel's output: what the program and its debugger printed,
//! selectable with the mouse and copyable, its `file:line` links opening
//! the code. An entity of its own, so the workspace only adds lines, places
//! it and opens the links it reports.

use std::ops::Range;

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::input::{Copy, SelectAll};
use gpui_kit::component::native_menu::NativeMenu;
use gpui_kit::component::v_flex;
use gpui_kit::*;

use jig_run::run_output::{Color, Link, OutputLine, Stream, Style};

/// The output panel's key context, where ⌘A selects all of it.
const OUTPUT_CONTEXT: &str = "JigRunOutput";

/// Select All in the output; Copy is bound for the whole window already.
pub(in crate::workspace) fn key_bindings() -> Vec<KeyBinding> {
    vec![KeyBinding::new(
        "secondary-a",
        SelectAll,
        Some(OUTPUT_CONTEXT),
    )]
}

/// Older lines are dropped past this many, so a chatty process can't eat
/// the machine's memory.
const MAX_LINES: usize = 50_000;
const LINE_HEIGHT: f32 = 18.;

/// The current run's output. One for the window, started afresh by each
/// run, so its focus stays put across runs.
pub(in crate::workspace) struct RunOutput {
    lines: Vec<OutputLine>,
    scroll: UniformListScrollHandle,
    selection: Option<OutputSelection>,
    /// Set while output is being selected with the mouse.
    selecting: bool,
    /// So ⌘C and ⌘A reach it once it's clicked.
    focus: FocusHandle,
}

/// A `file:line` link in the output was clicked.
pub(in crate::workspace) enum RunOutputEvent {
    OpenLink(Link),
}

impl EventEmitter<RunOutputEvent> for RunOutput {}

impl RunOutput {
    pub(in crate::workspace) fn new(cx: &mut Context<Self>) -> Self {
        Self {
            lines: Vec::new(),
            scroll: UniformListScrollHandle::new(),
            selection: None,
            selecting: false,
            focus: cx.focus_handle(),
        }
    }

    /// A new run's output, starting with `lines`.
    pub(in crate::workspace) fn start(&mut self, lines: Vec<OutputLine>, cx: &mut Context<Self>) {
        self.lines = lines;
        self.scroll = UniformListScrollHandle::new();
        self.selection = None;
        cx.notify();
    }

    pub(in crate::workspace) fn push(&mut self, line: OutputLine, cx: &mut Context<Self>) {
        self.lines.push(line);
        cx.notify();
    }

    pub(in crate::workspace) fn extend(
        &mut self,
        lines: impl IntoIterator<Item = OutputLine>,
        cx: &mut Context<Self>,
    ) {
        self.lines.extend(lines);
        cx.notify();
    }

    pub(in crate::workspace) fn len(&self) -> usize {
        self.lines.len()
    }

    #[cfg(test)]
    pub(in crate::workspace) fn lines(&self) -> &[OutputLine] {
        &self.lines
    }

    /// Whether the last line shows, so new output should keep it in view.
    pub(in crate::workspace) fn at_end(&self) -> bool {
        scrolled_to_end(&self.scroll)
    }

    /// Drop the oldest lines past the limit, the selection following.
    pub(in crate::workspace) fn trim(&mut self, cx: &mut Context<Self>) {
        if self.lines.len() > MAX_LINES {
            let excess = self.lines.len() - MAX_LINES + MAX_LINES / 10;
            self.lines.drain(..excess);
            self.selection = self
                .selection
                .and_then(|selection| selection.shifted_up(excess));
            cx.notify();
        }
    }

    /// Show the last line.
    pub(in crate::workspace) fn scroll_to_end(&self) {
        self.scroll
            .scroll_to_item(self.lines.len().saturating_sub(1), ScrollStrategy::Bottom);
    }

    /// Start selecting at the press, or extend the selection there with
    /// Shift. A double click takes the word, a triple the line.
    fn press(
        &mut self,
        point: (usize, usize),
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.focus(&self.focus, cx);
        let (line, at) = point;
        let text = self.lines.get(line).map_or("", |l| l.text.as_ref());
        self.selection = Some(match (event.click_count, self.selection) {
            (1, Some(selection)) if event.modifiers.shift => OutputSelection {
                head: point,
                ..selection
            },
            (2, _) => {
                let word = word_at(text, at);
                OutputSelection {
                    anchor: (line, word.start),
                    head: (line, word.end),
                }
            }
            (3.., _) => OutputSelection {
                anchor: (line, 0),
                head: (line, text.len()),
            },
            _ => OutputSelection {
                anchor: point,
                head: point,
            },
        });
        self.selecting = event.click_count == 1;
        cx.notify();
    }

    fn drag(&mut self, point: (usize, usize), dragging: bool, cx: &mut Context<Self>) {
        if !self.selecting {
            return;
        }
        if !dragging {
            self.selecting = false;
            return;
        }
        if let Some(selection) = self.selection.as_mut()
            && selection.head != point
        {
            selection.head = point;
            cx.notify();
        }
    }

    fn copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        let Some(selection) = self.selection.filter(|s| !s.is_empty()) else {
            return;
        };
        cx.write_to_clipboard(ClipboardItem::new_string(selection.text(&self.lines)));
    }

    fn select_all(&mut self, _: &SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        let last = self.lines.len().saturating_sub(1);
        let end = self.lines.last().map_or(0, |line| line.text.len());
        self.selection = Some(OutputSelection {
            anchor: (0, 0),
            head: (last, end),
        });
        cx.notify();
    }
}

impl Render for RunOutput {
    /// The output, focusable so it takes ⌘C and ⌘A, with a menu to copy it.
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let output = cx.entity().downgrade();
        let list = uniform_list(
            "run-output",
            self.lines.len(),
            cx.processor(move |this, range: Range<usize>, _, cx| {
                range
                    .map(|ix| {
                        let line = &this.lines[ix];
                        let selected = this
                            .selection
                            .and_then(|selection| selection.range_in(ix, line.text.len()));
                        render_line(line, ix, selected, output.clone(), cx)
                    })
                    .collect::<Vec<_>>()
            }),
        )
        .track_scroll(&self.scroll)
        .with_horizontal_sizing_behavior(ListHorizontalSizingBehavior::Unconstrained)
        .flex_1()
        .min_h_0()
        .px_3()
        .pb_1()
        .font_family(theme.mono_font_family.clone())
        .text_size(px(12.));
        v_flex()
            .id("run-output-area")
            .flex_1()
            .min_h_0()
            .key_context(OUTPUT_CONTEXT)
            .track_focus(&self.focus)
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(Self::select_all))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, _| this.selecting = false),
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(|this, event: &MouseDownEvent, window, cx| {
                    window.focus(&this.focus, cx);
                    let has_selection = this
                        .selection
                        .is_some_and(|selection| !selection.is_empty());
                    NativeMenu::new()
                        .menu_with_disabled("Copy", !has_selection, Box::new(Copy))
                        .menu("Select All", Box::new(SelectAll))
                        .show(event.position, window, cx);
                }),
            )
            .child(list)
    }
}

/// Whether the list shows its last line, so new output should keep it in
/// view; scrolled up to read, it stays put.
fn scrolled_to_end(scroll: &UniformListScrollHandle) -> bool {
    let base = scroll.0.borrow().base_handle.clone();
    -base.offset().y >= base.max_offset().y - px(2.)
}

/// Output selected with the mouse, from where the drag began to where it
/// is now. Points are a line and a byte offset in it.
#[derive(Clone, Copy, Debug, PartialEq)]
struct OutputSelection {
    anchor: (usize, usize),
    head: (usize, usize),
}

impl OutputSelection {
    fn ordered(self) -> ((usize, usize), (usize, usize)) {
        (self.anchor.min(self.head), self.anchor.max(self.head))
    }

    fn is_empty(self) -> bool {
        self.anchor == self.head
    }

    /// What's selected of line `ix`, `len` bytes long.
    fn range_in(self, ix: usize, len: usize) -> Option<Range<usize>> {
        let (start, end) = self.ordered();
        if ix < start.0 || ix > end.0 {
            return None;
        }
        let from = if ix == start.0 { start.1.min(len) } else { 0 };
        let to = if ix == end.0 { end.1.min(len) } else { len };
        (from < to).then_some(from..to)
    }

    /// The selected text, lines joined by newlines.
    fn text(self, lines: &[OutputLine]) -> String {
        let (start, end) = self.ordered();
        let mut text = String::new();
        for (ix, line) in lines.iter().enumerate().take(end.0 + 1).skip(start.0) {
            if ix > start.0 {
                text.push('\n');
            }
            if let Some(range) = self.range_in(ix, line.text.len()) {
                text.push_str(&line.text[range]);
            }
        }
        text
    }

    /// The same text after the first `count` lines were dropped, unless
    /// it was among them.
    fn shifted_up(self, count: usize) -> Option<Self> {
        let shift = |(line, at): (usize, usize)| Some((line.checked_sub(count)?, at));
        Some(Self {
            anchor: shift(self.anchor)?,
            head: shift(self.head)?,
        })
    }
}

/// The word around byte `at` of `text`: letters, digits and underscores,
/// or the single character there when it's none of those.
fn word_at(text: &str, at: usize) -> Range<usize> {
    let is_word = |c: char| c.is_alphanumeric() || c == '_';
    let at = at.min(text.len());
    // Past the end of the line, the last word counts as clicked.
    let at = match text[at..].chars().next() {
        Some(_) => at,
        None => text[..at].char_indices().last().map_or(at, |(i, _)| i),
    };
    let Some(here) = text[at..].chars().next() else {
        return at..at;
    };
    if !is_word(here) {
        return at..at + here.len_utf8();
    }
    let start = text[..at]
        .char_indices()
        .rev()
        .take_while(|&(_, c)| is_word(c))
        .last()
        .map_or(at, |(i, _)| i);
    let end = text[at..]
        .char_indices()
        .find(|&(_, c)| !is_word(c))
        .map_or(text.len(), |(i, _)| at + i);
    start..end
}

fn render_line(
    line: &OutputLine,
    ix: usize,
    selected: Option<Range<usize>>,
    output: WeakEntity<RunOutput>,
    cx: &App,
) -> AnyElement {
    let theme = cx.theme();
    let color = match line.stream {
        Stream::Meta => theme.muted_foreground,
        Stream::Stdout | Stream::Stderr => theme.foreground,
    };
    let link = HighlightStyle {
        color: Some(theme.link),
        underline: Some(UnderlineStyle {
            thickness: px(1.),
            color: Some(theme.link.opacity(0.6)),
            wavy: false,
        }),
        ..Default::default()
    };
    let spans: Vec<(Range<usize>, HighlightStyle)> = line
        .spans
        .iter()
        .map(|(range, style)| (range.clone(), highlight(*style, cx)))
        .collect();
    let links: Vec<(Range<usize>, HighlightStyle)> =
        line.links.iter().map(|l| (l.range.clone(), link)).collect();
    let selection = HighlightStyle {
        background_color: Some(theme.selection),
        ..Default::default()
    };
    let selected: Vec<(Range<usize>, HighlightStyle)> = selected
        .map(|range| (range, selection))
        .into_iter()
        .collect();
    let text = StyledText::new(line.text.clone())
        .with_highlights(layer(&layer(&spans, &links), &selected));
    let layout = text.layout().clone();
    let point_at = move |position: Point<Pixels>| {
        let at = layout
            .index_for_position(position)
            .unwrap_or_else(|closest| closest);
        (ix, at)
    };
    let content: AnyElement = if line.links.is_empty() {
        text.into_any_element()
    } else {
        let targets = line.links.clone();
        let output = output.clone();
        InteractiveText::new(("run-line", ix), text)
            .on_click(
                targets.iter().map(|l| l.range.clone()).collect(),
                move |clicked, _, cx| {
                    let target = targets[clicked].clone();
                    output
                        .update(cx, |_, cx| cx.emit(RunOutputEvent::OpenLink(target)))
                        .ok();
                },
            )
            .into_any_element()
    };
    let on_press = output.clone();
    let point_at_press = point_at.clone();
    div()
        .h(px(LINE_HEIGHT))
        .whitespace_nowrap()
        .text_color(color)
        .cursor_text()
        .on_mouse_down(MouseButton::Left, move |event, window, cx| {
            let point = point_at_press(event.position);
            on_press
                .update(cx, |this, cx| this.press(point, event, window, cx))
                .ok();
        })
        .on_mouse_move(move |event, _, cx| {
            let point = point_at(event.position);
            output
                .update(cx, |this, cx| this.drag(point, event.dragging(), cx))
                .ok();
        })
        .child(content)
        .into_any_element()
}

fn highlight(style: Style, cx: &App) -> HighlightStyle {
    let theme = cx.theme();
    HighlightStyle {
        color: style.fg.map(|fg| match fg {
            Color::Indexed(n) => match n % 8 {
                0 => theme.muted_foreground,
                1 => theme.red,
                2 => theme.green,
                3 => theme.yellow,
                4 => theme.blue,
                5 => theme.magenta,
                6 => theme.cyan,
                _ => theme.foreground,
            },
            Color::Rgb(r, g, b) => rgb(u32::from_be_bytes([0, r, g, b])).into(),
        }),
        font_weight: style.bold.then_some(FontWeight::BOLD),
        font_style: style.italic.then_some(FontStyle::Italic),
        underline: style.underline.then_some(UnderlineStyle {
            thickness: px(1.),
            color: None,
            wavy: false,
        }),
        fade_out: style.dim.then_some(0.4),
        ..Default::default()
    }
}

/// `top` over `base`, as the sorted, non-overlapping ranges text styling
/// needs. Both are sorted and non-overlapping themselves.
fn layer(
    base: &[(Range<usize>, HighlightStyle)],
    top: &[(Range<usize>, HighlightStyle)],
) -> Vec<(Range<usize>, HighlightStyle)> {
    if top.is_empty() {
        return base.to_vec();
    }
    let mut cuts: Vec<usize> = base
        .iter()
        .chain(top)
        .flat_map(|(range, _)| [range.start, range.end])
        .collect();
    cuts.sort_unstable();
    cuts.dedup();
    let style_at = |ranges: &[(Range<usize>, HighlightStyle)], at: usize| {
        ranges
            .iter()
            .find(|(range, _)| range.contains(&at))
            .map(|(_, style)| *style)
    };
    cuts.windows(2)
        .filter_map(|pair| {
            let (start, end) = (pair[0], pair[1]);
            let style = match (style_at(base, start), style_at(top, start)) {
                (None, None) => return None,
                (Some(base), None) => base,
                (None, Some(top)) => top,
                (Some(base), Some(top)) => base.highlight(top),
            };
            Some((start..end, style))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use gpui_kit::{HighlightStyle, Hsla};

    use super::{OutputSelection, layer, word_at};
    use jig_run::run_output::OutputLine;

    #[test]
    fn links_layer_over_colours() {
        let red = HighlightStyle {
            color: Some(Hsla::red()),
            ..Default::default()
        };
        let link = HighlightStyle {
            color: Some(Hsla::blue()),
            ..Default::default()
        };
        let layered = layer(&[(0..10, red)], &[(4..14, link)]);
        assert_eq!(layered, [(0..4, red), (4..10, link), (10..14, link)]);
    }

    #[test]
    fn selection_copies_across_lines_either_way() {
        let lines = ["first line", "second", "third line"].map(OutputLine::meta);
        let down = OutputSelection {
            anchor: (0, 6),
            head: (2, 5),
        };
        assert_eq!(down.text(&lines), "line\nsecond\nthird");
        let up = OutputSelection {
            anchor: down.head,
            head: down.anchor,
        };
        assert_eq!(up.text(&lines), "line\nsecond\nthird");
        assert_eq!(down.range_in(1, 6), Some(0..6));
        assert_eq!(down.range_in(3, 4), None);
    }

    #[test]
    fn selection_follows_dropped_lines() {
        let selection = OutputSelection {
            anchor: (5, 1),
            head: (7, 2),
        };
        assert_eq!(
            selection.shifted_up(5),
            Some(OutputSelection {
                anchor: (0, 1),
                head: (2, 2),
            })
        );
        assert_eq!(selection.shifted_up(6), None);
    }

    #[test]
    fn double_click_takes_the_word() {
        let text = "error: cannot_find foo";
        assert_eq!(&text[word_at(text, 10)], "cannot_find");
        assert_eq!(&text[word_at(text, 5)], ":");
        assert_eq!(&text[word_at(text, text.len())], "foo");
    }
}
