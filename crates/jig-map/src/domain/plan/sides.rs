//! A function's callers and callees, the packages outside, hubs and what's hidden.

use super::*;

/// Functions on one side of `of`, past `MAX_SIDE` folded into a group.
pub(super) fn side(
    plan: &mut Plan,
    project: &Project,
    of: &str,
    ids: Vec<FnId>,
    column: i32,
    opened: &Opened,
    queue: &mut Vec<FnId>,
) {
    let new: Vec<FnId> = ids
        .into_iter()
        .filter(|&id| plan.index_of(&function_key(project, id)).is_none())
        .collect();
    // Tests on a side fold into one group, unless it's opened or they're
    // all there is.
    let tests_key = format!("{of}@{column}:tests");
    let focus_is_test = plan
        .focus
        .and_then(|focus| plan.nodes[focus].function())
        .is_some_and(|id| project.is_test(id));
    let (tests, new): (Vec<FnId>, Vec<FnId>) = new
        .into_iter()
        .partition(|&id| project.is_test(id) && !focus_is_test);
    let (new, tests) = if opened.more.contains(&tests_key) || (new.is_empty() && tests.len() <= 2) {
        let mut all = new;
        all.extend(tests);
        (all, Vec::new())
    } else {
        (new, tests)
    };
    if !tests.is_empty() {
        let mut node = Node::new(
            format!("more:{tests_key}"),
            Item::More {
                ids: tests.clone(),
                key: tests_key,
            },
            format!(
                "{} test{}",
                tests.len(),
                if tests.len() == 1 { "" } else { "s" }
            ),
            column,
        );
        node.detail = "double-click to show them".into();
        plan.nodes.push(node);
    }
    let group = format!("{of}@{column}");
    let cap = if opened.more.contains(&group) {
        usize::MAX
    } else {
        MAX_SIDE
    };
    let (shown, rest) = if new.len() > cap {
        new.split_at(cap - 1)
    } else {
        (new.as_slice(), &[][..])
    };
    for &id in shown {
        add_function(plan, project, id, column);
        queue.push(id);
    }
    if !rest.is_empty() {
        let mut node = Node::new(
            format!("more:{group}"),
            Item::More {
                ids: rest.to_vec(),
                key: group,
            },
            format!("+{} more", rest.len()),
            column,
        );
        node.detail = rest
            .iter()
            .take(4)
            .map(|&id| project.functions[id].short().to_string())
            .collect::<Vec<_>>()
            .join(", ");
        plan.nodes.push(node);
    }
}

/// One node per package function `id` calls into, wired from it.
pub(super) fn externals(plan: &mut Plan, project: &Project, id: FnId, here: usize, column: i32) {
    let mut counts: Vec<(String, usize)> = Vec::new();
    for edge in &project.edges[id] {
        if let Target::External(package) = &edge.target {
            match counts.iter_mut().find(|(p, _)| p == package) {
                Some((_, count)) => *count += 1,
                None => counts.push((package.clone(), 1)),
            }
        }
    }
    for (package, count) in counts {
        let key = format!("ext:{package}");
        let ix = match plan.index_of(&key) {
            Some(ix) => ix,
            None => {
                let mut node = Node::new(key, Item::External(package.clone()), package, column);
                node.detail = "outside the project".into();
                plan.nodes.push(node);
                plan.nodes.len() - 1
            }
        };
        plan.wire(here, ix, WireKind::Call);
        let calls = plan.nodes[ix].detail.clone();
        let total = calls
            .split_whitespace()
            .next()
            .and_then(|n| n.parse::<usize>().ok())
            .unwrap_or(0)
            + count;
        plan.nodes[ix].detail = format!("{total} call{}", if total == 1 { "" } else { "s" });
    }
}

/// The calls between the functions drawn.
pub(super) fn wire_functions(plan: &mut Plan, project: &Project) {
    let index: HashMap<FnId, usize> = plan
        .nodes
        .iter()
        .enumerate()
        .filter_map(|(ix, node)| node.function().map(|id| (id, ix)))
        .collect();
    let mut wires = Vec::new();
    for (&id, &from) in &index {
        for edge in &project.edges[id] {
            match &edge.target {
                Target::Function(to) => {
                    if let Some(&to) = index.get(to) {
                        let kind = if edge.callback {
                            WireKind::Callback
                        } else {
                            WireKind::Call
                        };
                        wires.push((from, to, kind));
                    }
                }
                Target::Ambiguous(ids) => {
                    for to in ids {
                        if let Some(&to) = index.get(to) {
                            wires.push((from, to, WireKind::Ambiguous));
                        }
                    }
                }
                _ => {}
            }
        }
        // An interface's method leads to its implementations drawn.
        for to in project.implementations(id) {
            if let Some(&to) = index.get(&to) {
                wires.push((from, to, WireKind::Ambiguous));
            }
        }
    }
    wires.sort_by_key(|(from, to, _)| (*from, *to));
    for (from, to, kind) in wires {
        let solid = plan
            .wires
            .iter()
            .any(|w| w.from == from && w.to == to && w.kind == WireKind::Call);
        if !(solid && kind == WireKind::Ambiguous) {
            plan.wire(from, to, kind);
        }
    }
}

/// Hubs each drawn function calls, as badges with how often.
pub(super) fn badge_hubs(plan: &mut Plan, project: &Project) {
    let focus = plan.focus.and_then(|ix| plan.nodes[ix].function());
    for node in &mut plan.nodes {
        let Some(id) = node.function() else {
            continue;
        };
        let mut counts: Vec<(FnId, usize)> = Vec::new();
        for edge in &project.edges[id] {
            if let Target::Function(to) = edge.target
                && project.is_hub(to)
                && Some(to) != focus
            {
                match counts.iter_mut().find(|(t, _)| *t == to) {
                    Some((_, n)) => *n += 1,
                    None => counts.push((to, 1)),
                }
            }
        }
        node.badges = counts
            .into_iter()
            .map(|(to, n)| {
                let name = project.functions[to].short();
                if n > 1 {
                    format!("{name} ×{n}")
                } else {
                    name.to_string()
                }
            })
            .collect();
    }
}

/// For each function drawn, how many of its callers and callees aren't.
pub(super) fn count_hidden(plan: &mut Plan, project: &Project) {
    let drawn: HashSet<FnId> = plan
        .nodes
        .iter()
        .filter_map(Node::function)
        .chain(plan.nodes.iter().flat_map(|node| match &node.item {
            Item::More { ids, .. } => ids.clone(),
            _ => Vec::new(),
        }))
        .collect();
    for node in &mut plan.nodes {
        let Some(id) = node.function() else {
            continue;
        };
        node.hidden_callers = project.callers[id]
            .iter()
            .filter(|c| !drawn.contains(c))
            .count();
        node.hidden_callees = project
            .callees(id)
            .into_iter()
            .filter(|c| !drawn.contains(c) && !project.is_hub(*c))
            .count();
        node.implementations = project
            .implementations(id)
            .into_iter()
            .filter(|c| !drawn.contains(c))
            .count();
    }
}
