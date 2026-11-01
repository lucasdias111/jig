//! Site's arrows: calls drawn between the siblings holding their ends.

use super::*;

/// The calls between files and types: each certain call (or callback)
/// from a function in one to a function in another, counted.
pub(super) fn calls(
    project: &Project,
    leaves: &HashMap<(usize, Option<String>), usize>,
) -> HashMap<(usize, usize), Calls> {
    let unit = |id: usize| {
        let function = &project.functions[id];
        leaves
            .get(&(function.file, function.outline.owner.clone()))
            .or_else(|| leaves.get(&(function.file, None)))
            .copied()
    };
    let mut calls: HashMap<(usize, usize), Calls> = HashMap::new();
    for (id, edges) in project.edges.iter().enumerate() {
        let Some(from) = unit(id) else {
            continue;
        };
        for edge in edges {
            let Target::Function(to) = edge.target else {
                continue;
            };
            let Some(into) = unit(to) else {
                continue;
            };
            if from == into {
                continue;
            }
            let entry = calls.entry((from, into)).or_default();
            entry.count += 1;
            let example = format!(
                "{} → {}",
                project.functions[id].title(),
                project.functions[to].title()
            );
            if entry.examples.len() < MAX_EXAMPLES && !entry.examples.contains(&example) {
                entry.examples.push(example);
            }
        }
    }
    calls
}

impl Site {
    /// The arrows between the boxes showing, `shown` from `shown_as`: each
    /// call inside a region drawn between the two boxes side by side that
    /// hold its ends, summed, one arrow per direction. Calls between
    /// regions aren't drawn: the domains' links say what goes between
    /// them. Most calls first.
    pub fn arrows(&self, shown: &[usize]) -> Vec<Arrow> {
        let mut summed: HashMap<(usize, usize), Calls> = HashMap::new();
        for (&(from, to), calls) in &self.calls {
            if self.nodes[from].region != self.nodes[to].region {
                continue;
            }
            let (a, b) = (shown[from], shown[to]);
            if a == b {
                continue;
            }
            let Some((a, b)) = self.side_by_side(a, b) else {
                continue;
            };
            summed.entry((a, b)).or_default().add(calls);
        }
        let mut arrows: Vec<Arrow> = summed
            .into_iter()
            .map(|((from, to), calls)| Arrow { from, to, calls })
            .collect();
        arrows.sort_by(|a, b| {
            b.calls
                .count
                .cmp(&a.calls.count)
                .then((a.from, a.to).cmp(&(b.from, b.to)))
        });
        arrows
    }

    /// The arrows, as `arrows` gives them for `shown`, that carry a call
    /// into or out of node `chosen` or what's inside it.
    pub fn arrows_of(&self, chosen: usize, shown: &[usize]) -> Vec<(usize, usize)> {
        let inside = |ix: usize| self.ancestry(ix).any(|n| n == chosen);
        let mut found = Vec::new();
        for &(from, to) in self.calls.keys() {
            if self.nodes[from].region != self.nodes[to].region || !(inside(from) || inside(to)) {
                continue;
            }
            if let Some(pair) = self.side_by_side(shown[from], shown[to])
                && !found.contains(&pair)
            {
                found.push(pair);
            }
        }
        found
    }

    /// The two boxes side by side, in the box holding both, that hold `a`
    /// and `b`; none when one holds the other.
    pub(super) fn side_by_side(&self, a: usize, b: usize) -> Option<(usize, usize)> {
        let up: Vec<usize> = self.ancestry(a).collect();
        let mut below_b = b;
        for n in self.ancestry(b) {
            if let Some(at) = up.iter().position(|&m| m == n) {
                if at == 0 || n == b {
                    return None;
                }
                return Some((up[at - 1], below_b));
            }
            below_b = n;
        }
        None
    }
}
