//! What the debugger shows on a blueprint: which blocks have breakpoints,
//! the one it's paused in, and the path it took getting there. The
//! workspace hands over places in the file; this finds their blocks.

use std::collections::{HashSet, VecDeque};
use std::ops::Range;

use super::{Graph, node_at};

/// The debugger's marks in the file shown, as byte offsets: each at the
/// first code on its line.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DebugMarks {
    /// Breakpoints can be set: the file's debugger is on in Settings.
    pub enabled: bool,
    pub breakpoints: Vec<usize>,
    /// Where it's paused, if it's paused here.
    pub paused: Option<usize>,
    /// Where it paused this session, oldest first.
    pub trail: Vec<usize>,
}

/// The marks on the blocks of one function.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OnBlocks {
    pub breakpoints: HashSet<usize>,
    pub paused: Option<usize>,
    /// How many times it paused in each block, by node.
    pub visits: Vec<usize>,
    /// The exec wires it went along, by edge.
    pub taken: HashSet<usize>,
}

impl OnBlocks {
    /// Whether it has paused in this function at all, so blocks it hasn't
    /// reached can be shown as such.
    pub fn traced(&self) -> bool {
        self.visits.iter().any(|&visits| visits > 0)
    }
}

/// The marks on `graph`'s blocks, the function at `function` in the file.
pub fn on_blocks(graph: &Graph, function: Range<usize>, marks: &DebugMarks) -> OnBlocks {
    let block = |offset: usize| node_at(graph, offset);
    let mut visits = vec![0; graph.nodes.len()];
    let mut taken = HashSet::new();
    let mut last = None;
    for &offset in &marks.trail {
        let Some(at) = block(offset) else {
            // Gone into another function: the path here starts again. A
            // line of this one with no block, a closing `}`, is on the way.
            if !function.contains(&offset) {
                last = None;
            }
            continue;
        };
        // Steps within one block, a statement over several lines, are one.
        if last == Some(at) {
            continue;
        }
        visits[at] += 1;
        if let Some(from) = last {
            taken.extend(way(graph, from, at));
        }
        last = Some(at);
    }
    OnBlocks {
        breakpoints: marks
            .breakpoints
            .iter()
            .filter_map(|&at| block(at))
            .collect(),
        paused: marks.paused.and_then(block),
        visits,
        taken,
    }
}

/// The wires from node `from` to node `to`: straight there, or round
/// the back of the innermost loop around `from` that leads there, since
/// a loop's way back isn't drawn.
fn way(graph: &Graph, from: usize, to: usize) -> Vec<usize> {
    if let Some(edges) = path(graph, from, to) {
        return edges;
    }
    (0..from)
        .rev()
        .filter(|&node| graph.nodes[node].kind == super::Kind::Loop)
        .filter(|&node| path(graph, node, from).is_some())
        .find_map(|node| path(graph, node, to))
        .unwrap_or_default()
}

/// The edges of the shortest way from node `from` to node `to`, if the
/// flow goes there.
fn path(graph: &Graph, from: usize, to: usize) -> Option<Vec<usize>> {
    let mut came_by: Vec<Option<usize>> = vec![None; graph.nodes.len()];
    let mut seen = vec![false; graph.nodes.len()];
    let mut queue = VecDeque::from([from]);
    seen[from] = true;
    while let Some(node) = queue.pop_front() {
        if node == to {
            let mut edges = Vec::new();
            let mut at = to;
            while let Some(edge) = came_by[at] {
                edges.push(edge);
                at = graph.edges[edge].from;
            }
            return Some(edges);
        }
        for (ix, edge) in graph.edges.iter().enumerate() {
            if edge.from == node && !seen[edge.to] {
                seen[edge.to] = true;
                came_by[edge.to] = Some(ix);
                queue.push_back(edge.to);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::blueprint::{functions, graph};

    const SOURCE: &str = "fn go(items: Vec<u8>) {\n    let total = 0;\n    for item in items {\n        if item > 2 {\n            big(item);\n        } else {\n            small(item);\n        }\n    }\n    done(total);\n}\n";

    fn at(text: &str) -> usize {
        SOURCE.find(text).unwrap()
    }

    fn flow() -> Graph {
        graph(SOURCE, &functions(SOURCE)[0]).unwrap()
    }

    fn function() -> Range<usize> {
        functions(SOURCE)[0].range.clone()
    }

    fn title(graph: &Graph, node: usize) -> &str {
        &graph.nodes[node].title
    }

    #[test]
    fn breakpoints_and_the_pause_find_their_blocks() {
        let graph = flow();
        let marks = DebugMarks {
            enabled: true,
            breakpoints: vec![at("big("), 0],
            paused: Some(at("done(")),
            trail: Vec::new(),
        };
        let blocks = on_blocks(&graph, function(), &marks);
        let mut breakpoints: Vec<_> = blocks
            .breakpoints
            .iter()
            .map(|&n| title(&graph, n))
            .collect();
        breakpoints.sort();
        // The first is in `big`; offset 0 is the function's own line.
        assert_eq!(breakpoints, ["Call big", "go"]);
        assert_eq!(title(&graph, blocks.paused.unwrap()), "Call done");
        assert!(!blocks.traced());
    }

    #[test]
    fn the_trail_counts_visits_and_lights_the_way_taken() {
        let graph = flow();
        let trail = vec![
            at("let total"),
            at("for item"),
            at("if item"),
            at("big("),
            // Round the loop again, the other way.
            at("for item"),
            at("if item"),
            at("small("),
            // LLDB stops on the loop's closing brace on the way out.
            at("    }\n    done") + 4,
            at("done("),
        ];
        let marks = DebugMarks {
            trail,
            ..Default::default()
        };
        let blocks = on_blocks(&graph, function(), &marks);
        let visits = |name: &str| {
            let node = graph.nodes.iter().position(|n| n.title == name).unwrap();
            blocks.visits[node]
        };
        assert_eq!(visits("For Each"), 2);
        assert_eq!(visits("Branch"), 2);
        assert_eq!(visits("Call big"), 1);
        assert_eq!(visits("Call small"), 1);
        assert_eq!(visits("go"), 0);
        let taken: HashSet<(&str, &str)> = blocks
            .taken
            .iter()
            .map(|&edge| {
                let edge = graph.edges[edge];
                (title(&graph, edge.from), title(&graph, edge.to))
            })
            .collect();
        assert!(taken.contains(&("Branch", "Call big")));
        assert!(taken.contains(&("Branch", "Call small")));
        assert!(taken.contains(&("For Each", "Call done")));
        // From `big` back to the loop is round the back, not drawn.
        assert!(!taken.contains(&("Call big", "For Each")));
    }

    #[test]
    fn leaving_the_function_starts_the_path_again() {
        let graph = flow();
        let marks = DebugMarks {
            trail: vec![at("let total"), SOURCE.len() + 10, at("done(")],
            ..Default::default()
        };
        let blocks = on_blocks(&graph, function(), &marks);
        assert!(blocks.taken.is_empty());
        assert!(blocks.traced());
    }
}
