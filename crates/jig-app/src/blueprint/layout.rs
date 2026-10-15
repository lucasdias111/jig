//! Where each block of a flow goes, in unzoomed points: columns left to
//! right by how many steps in a block is, each path kept level with the
//! step before it, and a branch's later outputs set below its first.
//! Data pins, when shown, are rows of their own below a block's code:
//! inputs down the left, values down the right after its named outputs.

use super::{Graph, Node};

pub const NODE_WIDTH: f32 = 230.;
pub const HEADER_HEIGHT: f32 = 26.;
pub const LINE_HEIGHT: f32 = 17.;
pub const PIN_HEIGHT: f32 = 20.;
/// Padding above and below a block's code and pins.
pub const BODY_PADDING: f32 = 6.;
const COLUMN_GAP: f32 = 70.;
const ROW_GAP: f32 = 28.;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Rect {
    pub fn right(&self) -> f32 {
        self.x + self.width
    }

    pub fn bottom(&self) -> f32 {
        self.y + self.height
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Layout {
    /// One per node, in the graph's order.
    pub rects: Vec<Rect>,
    /// The whole drawing's size.
    pub width: f32,
    pub height: f32,
    /// Whether blocks have rows for their data pins.
    pub data: bool,
}

impl Layout {
    /// Where output `out` of node `ix` leaves it.
    pub fn out_pin(&self, graph: &Graph, ix: usize, out: usize) -> (f32, f32) {
        let rect = self.rects[ix];
        (
            rect.right(),
            rect.y + out_offset(graph.nodes[ix].outs.len(), code_lines(graph, ix), out),
        )
    }

    /// Where the exec wire comes into node `ix`.
    pub fn in_pin(&self, ix: usize) -> (f32, f32) {
        let rect = self.rects[ix];
        (rect.x, rect.y + HEADER_HEIGHT / 2.)
    }

    /// Where data output `value` of node `ix` leaves it, below its named
    /// outputs.
    pub fn value_pin(&self, graph: &Graph, ix: usize, value: usize) -> (f32, f32) {
        let rect = self.rects[ix];
        let node = &graph.nodes[ix];
        let row = exec_rows(node.outs.len()) + value;
        (rect.right(), rect.y + row_offset(node.code.len(), row))
    }

    /// Where data input `input` comes into node `ix`.
    pub fn input_pin(&self, graph: &Graph, ix: usize, input: usize) -> (f32, f32) {
        let rect = self.rects[ix];
        (
            rect.x,
            rect.y + row_offset(graph.nodes[ix].code.len(), input),
        )
    }
}

fn code_lines(graph: &Graph, ix: usize) -> usize {
    graph.nodes[ix].code.len()
}

/// Whether a node's outputs are listed by name below its code, rather than
/// one plain output in its header.
pub fn named_outs(outs: usize) -> bool {
    outs > 1
}

/// The rows a node's named outputs take on its right.
fn exec_rows(outs: usize) -> usize {
    if named_outs(outs) { outs } else { 0 }
}

/// The pin rows below a node's code: its longer side.
pub fn pin_rows(node: &Node, data: bool) -> usize {
    let (inputs, values) = if data {
        (node.inputs.len(), node.values.len())
    } else {
        (0, 0)
    };
    inputs.max(exec_rows(node.outs.len()) + values)
}

/// How far below a node's top the middle of pin row `row` is.
fn row_offset(code_lines: usize, row: usize) -> f32 {
    HEADER_HEIGHT
        + BODY_PADDING
        + code_lines as f32 * LINE_HEIGHT
        + row as f32 * PIN_HEIGHT
        + PIN_HEIGHT / 2.
}

/// How far below a node's top output `out` is.
pub fn out_offset(outs: usize, code_lines: usize, out: usize) -> f32 {
    if !named_outs(outs) {
        return HEADER_HEIGHT / 2.;
    }
    row_offset(code_lines, out)
}

pub fn node_height(node: &Node, data: bool) -> f32 {
    let body = node.code.len() as f32 * LINE_HEIGHT + pin_rows(node, data) as f32 * PIN_HEIGHT;
    if body == 0. {
        HEADER_HEIGHT
    } else {
        HEADER_HEIGHT + body + 2. * BODY_PADDING
    }
}

/// With `data`, blocks make room for their data pins.
pub fn layout(graph: &Graph, data: bool) -> Layout {
    let count = graph.nodes.len();
    let heights: Vec<f32> = graph
        .nodes
        .iter()
        .map(|node| node_height(node, data))
        .collect();
    // Edges only run forward, so one pass in order finds each column.
    let mut columns = vec![0usize; count];
    for edge in &graph.edges {
        columns[edge.to] = columns[edge.to].max(columns[edge.from] + 1);
    }
    let mut rects = vec![Rect::default(); count];
    let mut placed = vec![false; count];
    let column_count = columns.iter().max().map_or(0, |max| max + 1);
    for column in 0..column_count {
        let mut members: Vec<(f32, usize)> = (0..count)
            .filter(|&ix| columns[ix] == column)
            .map(|ix| (wanted_y(graph, &rects, &placed, &heights, ix), ix))
            .collect();
        members.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
        let mut below = f32::NEG_INFINITY;
        for (wanted, ix) in members {
            let y = wanted.max(below);
            rects[ix] = Rect {
                x: column as f32 * (NODE_WIDTH + COLUMN_GAP),
                y,
                width: NODE_WIDTH,
                height: heights[ix],
            };
            placed[ix] = true;
            below = y + heights[ix] + ROW_GAP;
        }
    }
    let width = rects.iter().map(Rect::right).fold(0., f32::max);
    let height = rects.iter().map(Rect::bottom).fold(0., f32::max);
    Layout {
        rects,
        width,
        height,
        data,
    }
}

/// Level with the step it comes from, or for a branch's later outputs,
/// that far below it; where several paths meet, the highest.
fn wanted_y(graph: &Graph, rects: &[Rect], placed: &[bool], heights: &[f32], ix: usize) -> f32 {
    graph
        .edges
        .iter()
        .filter(|edge| edge.to == ix && placed[edge.from])
        .map(|edge| {
            let from = rects[edge.from];
            if edge.out == 0 {
                from.y
            } else {
                // Below the block, a row per output before it.
                from.y + heights[edge.from] + ROW_GAP + (edge.out - 1) as f32 * ROW_GAP
            }
        })
        .fold(None, |highest: Option<f32>, y| {
            Some(highest.map_or(y, |highest| highest.min(y)))
        })
        .unwrap_or(0.)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::blueprint::{functions, graph};

    fn laid_out(source: &str) -> (Graph, Layout) {
        let graph = graph(source, &functions(source)[0]).unwrap();
        let layout = layout(&graph, true);
        (graph, layout)
    }

    fn overlap(a: Rect, b: Rect) -> bool {
        a.x < b.right() && b.x < a.right() && a.y < b.bottom() && b.y < a.bottom()
    }

    #[test]
    fn a_straight_flow_is_one_row() {
        let (_, layout) = laid_out("fn go() { a(); b(); c(); }");
        let ys: Vec<f32> = layout.rects.iter().map(|rect| rect.y).collect();
        assert_eq!(ys, [0., 0., 0., 0.]);
        assert!(
            layout
                .rects
                .windows(2)
                .all(|pair| pair[0].right() < pair[1].x)
        );
    }

    #[test]
    fn a_branch_puts_false_below_true() {
        let (_, layout) = laid_out("fn go() { if x { a(); } else { b(); } c(); }");
        let (branch, yes, no, after) = (
            layout.rects[1],
            layout.rects[2],
            layout.rects[3],
            layout.rects[4],
        );
        assert_eq!(yes.y, branch.y);
        assert!(no.y >= yes.bottom());
        // Where they meet, level with True.
        assert_eq!(after.y, yes.y);
        assert!(after.x > yes.right());
    }

    #[test]
    fn blocks_never_overlap() {
        let (_, layout) = laid_out(
            "fn go(v: u8) { match v { 1 => a(), 2 => { b(); c(); } _ => { if d { e(); } } } for i in x { f(i); } g(); }",
        );
        for (i, a) in layout.rects.iter().enumerate() {
            for b in &layout.rects[i + 1..] {
                assert!(!overlap(*a, *b), "{a:?} overlaps {b:?}");
            }
        }
    }

    #[test]
    fn pins_sit_on_their_rows() {
        let (graph, layout) = laid_out("fn go() { if x { a(); } }");
        let (_, true_y) = layout.out_pin(&graph, 1, 0);
        let (_, false_y) = layout.out_pin(&graph, 1, 1);
        assert_eq!(false_y - true_y, PIN_HEIGHT);
        let branch = layout.rects[1];
        assert!(true_y > branch.y + HEADER_HEIGHT && false_y < branch.bottom());
        // A plain output is in the header.
        assert_eq!(layout.out_pin(&graph, 0, 0).1, HEADER_HEIGHT / 2.);
    }

    #[test]
    fn data_pins_keep_clear_of_exec_pins() {
        let (graph, layout) = laid_out(
            "fn go(a: u8, b: u8) { let c = a + b; if c > a { let d = c; use_it(d, b); } else { other(a, b, c); } match c { 1 => a, _ => b } }",
        );
        assert!(!graph.data.is_empty());
        for (ix, node) in graph.nodes.iter().enumerate() {
            let rect = layout.rects[ix];
            let mut left = vec![layout.in_pin(ix)];
            left.extend((0..node.inputs.len()).map(|input| layout.input_pin(&graph, ix, input)));
            let mut right: Vec<_> = (0..node.outs.len())
                .map(|out| layout.out_pin(&graph, ix, out))
                .collect();
            right.extend((0..node.values.len()).map(|value| layout.value_pin(&graph, ix, value)));
            for side in [left, right] {
                for (i, a) in side.iter().enumerate() {
                    assert!(a.1 > rect.y && a.1 < rect.bottom(), "{a:?} is off {rect:?}");
                    for b in &side[i + 1..] {
                        assert_eq!(a.0, b.0);
                        assert!(
                            (a.1 - b.1).abs() >= PIN_HEIGHT,
                            "{a:?} and {b:?} on {}",
                            node.title
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn hiding_data_drops_its_rows() {
        let graph = graph(
            "fn go(a: u8) { use_it(a); }",
            &functions("fn go(a: u8) { use_it(a); }")[0],
        )
        .unwrap();
        let shown = layout(&graph, true);
        let hidden = layout(&graph, false);
        assert!(hidden.rects[1].height < shown.rects[1].height);
    }
}
