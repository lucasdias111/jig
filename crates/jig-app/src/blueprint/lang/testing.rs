//! What every language's tests need: a function's flow, its blocks'
//! titles, where an output leads, and its data wires.

use crate::blueprint::{Graph, Lang, functions, graph};

/// The flow of the first function in `source`.
pub fn flow(source: &str, lang: Lang) -> Graph {
    let functions = functions(source, lang);
    graph(source, lang, &functions[0]).expect("the function is there")
}

/// The flow of the function called `name` in `source`.
pub fn flow_of(source: &str, lang: Lang, name: &str) -> Graph {
    let functions = functions(source, lang);
    let function = functions
        .iter()
        .find(|function| function.name == name)
        .unwrap_or_else(|| panic!("no {name} in {functions:?}"));
    graph(source, lang, function).expect("the function is there")
}

pub fn titles(graph: &Graph) -> Vec<&str> {
    graph.nodes.iter().map(|node| node.title.as_str()).collect()
}

/// The nodes output `out` of `from` leads to.
pub fn next(graph: &Graph, from: usize, out: usize) -> Vec<usize> {
    graph
        .edges
        .iter()
        .filter(|edge| edge.from == from && edge.out == out)
        .map(|edge| edge.to)
        .collect()
}

/// The first block titled `title`.
pub fn node(graph: &Graph, title: &str) -> usize {
    graph
        .nodes
        .iter()
        .position(|node| node.title == title)
        .unwrap_or_else(|| panic!("no {title} in {:?}", titles(graph)))
}

/// Where the block titled `from` leads through output `out`, by title.
pub fn after<'g>(graph: &'g Graph, from: &str, out: usize) -> Vec<&'g str> {
    next(graph, node(graph, from), out)
        .into_iter()
        .map(|to| graph.nodes[to].title.as_str())
        .collect()
}

/// Each data wire: (binding block, name, reading block).
pub fn wires(graph: &Graph) -> Vec<(&str, &str, &str)> {
    graph
        .data
        .iter()
        .map(|data| {
            (
                graph.nodes[data.from].title.as_str(),
                graph.nodes[data.from].values[data.value].as_str(),
                graph.nodes[data.to].title.as_str(),
            )
        })
        .collect()
}

/// Every wire runs from an earlier block to a later one.
pub fn runs_forward(graph: &Graph) -> bool {
    graph.edges.iter().all(|edge| edge.from < edge.to)
}
