//! Code firing off while it runs, as in Unreal: each block the program
//! passes through lights up and fades, and so does the wire it came
//! along, with a pulse running down it. The debugger reports each block's
//! first line as it's reached, without stopping (a logpoint); this keeps
//! when each block and wire last fired.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use super::Graph;
use super::debug::way;

/// How long a block or wire stays lit after firing.
pub const FADE: Duration = Duration::from_millis(1200);
/// How long a pulse takes to run down a wire.
pub const PULSE: Duration = Duration::from_millis(350);

#[derive(Clone, Debug, Default)]
pub struct Firing {
    /// When each block last fired, by node.
    blocks: HashMap<usize, Instant>,
    /// When each exec wire was last taken, by edge.
    wires: HashMap<usize, Instant>,
    /// The block that fired last, which the next one came from.
    last: Option<usize>,
}

impl Firing {
    /// Block `node` of `graph` fired at `now`. The start begins the
    /// function again; anything else came along the way from the last.
    pub fn fire(&mut self, graph: &Graph, node: usize, now: Instant) {
        if node != 0
            && let Some(from) = self.last
            && from != node
        {
            for edge in way(graph, from, node) {
                self.wires.insert(edge, now);
            }
        }
        self.blocks.insert(node, now);
        self.last = Some(node);
    }

    /// Another function is drawn: nothing of it has fired.
    pub fn clear(&mut self) {
        *self = Self::default();
    }

    /// How brightly block `node` shows, from 1 as it fires to 0.
    pub fn block(&self, node: usize, now: Instant) -> f32 {
        strength(self.blocks.get(&node), now)
    }

    /// How brightly wire `edge` shows, from 1 as it's taken to 0, and how
    /// far down it the pulse is, while it runs.
    pub fn wire(&self, edge: usize, now: Instant) -> (f32, Option<f32>) {
        let Some(&at) = self.wires.get(&edge) else {
            return (0., None);
        };
        let age = now.saturating_duration_since(at);
        let pulse = (age < PULSE).then(|| age.as_secs_f32() / PULSE.as_secs_f32());
        (strength(Some(&at), now), pulse)
    }

    /// Whether anything is still lit, so the view keeps drawing.
    pub fn lit(&self, now: Instant) -> bool {
        self.blocks
            .values()
            .chain(self.wires.values())
            .any(|&at| now.saturating_duration_since(at) < FADE)
    }
}

fn strength(at: Option<&Instant>, now: Instant) -> f32 {
    let Some(&at) = at else {
        return 0.;
    };
    let age = now.saturating_duration_since(at).as_secs_f32();
    (1. - age / FADE.as_secs_f32()).max(0.)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::blueprint::{functions, graph};

    const SOURCE: &str = "fn go(items: Vec<u8>) {\n    for item in items {\n        if item > 2 {\n            big(item);\n        } else {\n            small(item);\n        }\n    }\n    done();\n}\n";

    fn node(graph: &Graph, title: &str) -> usize {
        graph.nodes.iter().position(|n| n.title == title).unwrap()
    }

    fn edge(graph: &Graph, from: &str, to: &str) -> usize {
        let (from, to) = (node(graph, from), node(graph, to));
        graph
            .edges
            .iter()
            .position(|e| e.from == from && e.to == to)
            .unwrap()
    }

    #[test]
    fn lights_the_blocks_and_the_wires_between_them_then_fades() {
        let graph = graph(SOURCE, &functions(SOURCE)[0]).unwrap();
        let mut firing = Firing::default();
        let start = Instant::now();
        firing.fire(&graph, 0, start);
        for title in ["For Each", "Branch", "Call big"] {
            firing.fire(&graph, node(&graph, title), start);
        }
        assert_eq!(firing.block(node(&graph, "Call big"), start), 1.);
        assert_eq!(firing.block(node(&graph, "Call small"), start), 0.);
        let (lit, pulse) = firing.wire(edge(&graph, "Branch", "Call big"), start);
        assert_eq!((lit, pulse), (1., Some(0.)));
        assert_eq!(
            firing.wire(edge(&graph, "Branch", "Call small"), start).0,
            0.
        );
        assert!(firing.lit(start));

        let later = start + PULSE;
        assert_eq!(
            firing.wire(edge(&graph, "Branch", "Call big"), later).1,
            None
        );
        let gone = start + FADE;
        assert_eq!(firing.block(node(&graph, "Call big"), gone), 0.);
        assert!(!firing.lit(gone));
    }

    #[test]
    fn round_the_loop_lights_the_way_back_and_the_start_begins_again() {
        let graph = graph(SOURCE, &functions(SOURCE)[0]).unwrap();
        let mut firing = Firing::default();
        let now = Instant::now();
        for title in ["For Each", "Branch", "Call big", "Branch", "Call small"] {
            firing.fire(&graph, node(&graph, title), now);
        }
        assert_eq!(firing.wire(edge(&graph, "Branch", "Call small"), now).0, 1.);
        // A new call of the function: no wire from where the last one was.
        let mut firing = Firing::default();
        firing.fire(&graph, node(&graph, "Call done"), now);
        firing.fire(&graph, 0, now);
        assert!(
            graph
                .edges
                .iter()
                .enumerate()
                .all(|(ix, _)| firing.wire(ix, now).0 == 0.)
        );
    }
}
