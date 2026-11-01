//! Spreading a Plan's boxes out around its focus, and opening further hops.

use super::*;

/// Around a module's functions, in columns 0 to `deepest`: its callers
/// from elsewhere left of them, what they call elsewhere right of them.
pub(super) fn spread_around(
    plan: &mut Plan,
    project: &Project,
    of: &str,
    centre: &[FnId],
    deepest: i32,
    opened: &Opened,
) {
    let inside: HashSet<FnId> = centre.iter().copied().collect();
    let mut callees = Vec::new();
    let mut callers = Vec::new();
    for &id in centre {
        for to in project.callees(id) {
            if !inside.contains(&to) && !project.is_hub(to) && !callees.contains(&to) {
                callees.push(to);
            }
        }
        for &from in &project.callers[id] {
            if !inside.contains(&from) && !callers.contains(&from) {
                callers.push(from);
            }
        }
    }
    let mut queue = Vec::new();
    side(plan, project, of, callees, deepest + 1, opened, &mut queue);
    for &id in centre {
        if let Some(here) = plan.index_of(&function_key(project, id)) {
            externals(plan, project, id, here, deepest + 1);
        }
    }
    side(plan, project, of, callers, -1, opened, &mut queue);
    open_hops(plan, project, queue, centre, None, opened);
}

/// The callers of `centre` (its functions, drawn in column 0) on the
/// left and their callees on the right, as one side each named `of`, then
/// each hop opened past them.
pub(super) fn spread(
    plan: &mut Plan,
    project: &Project,
    of: &str,
    centre: &[FnId],
    opened: &Opened,
) {
    let mut queue = Vec::new();
    let inside = |id: &FnId| centre.contains(id);
    let mut callees = Vec::new();
    let mut callers = Vec::new();
    let focus = (centre.len() == 1).then(|| centre[0]);
    for &id in centre {
        for to in project.callees(id) {
            if !inside(&to) && (!project.is_hub(to) || Some(to) == focus) && !callees.contains(&to)
            {
                callees.push(to);
            }
        }
        for &from in &project.callers[id] {
            if !inside(&from) && !callers.contains(&from) {
                callers.push(from);
            }
        }
    }
    side(plan, project, of, callees, 1, opened, &mut queue);
    for &id in centre {
        let Some(here) = plan.index_of(&function_key(project, id)) else {
            continue;
        };
        // Calls whose target is one of a few by name: drawn dashed.
        for edge in &project.edges[id] {
            if let Target::Ambiguous(ids) = &edge.target {
                for &to in ids {
                    if !inside(&to) {
                        add_function(plan, project, to, 1);
                    }
                }
            }
        }
        externals(plan, project, id, here, 1);
    }
    side(plan, project, of, callers, -1, opened, &mut queue);
    open_hops(
        plan,
        project,
        std::mem::take(&mut queue),
        centre,
        focus,
        opened,
    );
    // The centre's own hops opened, as a function's focus opens them.
    for &id in centre {
        let key = function_key(project, id);
        if opened.implementations.contains(&key) {
            let implementations = project.implementations(id);
            side(plan, project, &key, implementations, 1, opened, &mut queue);
        }
    }
}

/// Each hop opened past the functions in `queue`, and past those.
pub(super) fn open_hops(
    plan: &mut Plan,
    project: &Project,
    mut queue: Vec<FnId>,
    centre: &[FnId],
    focus: Option<FnId>,
    opened: &Opened,
) {
    let mut done: HashSet<FnId> = centre.iter().copied().collect();
    while let Some(id) = queue.pop() {
        if !done.insert(id) {
            continue;
        }
        let key = function_key(project, id);
        let Some(here) = plan.index_of(&key) else {
            continue;
        };
        let column = plan.nodes[here].column;
        if opened.callees.contains(&key) {
            let callees: Vec<FnId> = project
                .callees(id)
                .into_iter()
                .filter(|&to| !project.is_hub(to) || Some(to) == focus)
                .collect();
            side(plan, project, &key, callees, column + 1, opened, &mut queue);
            for edge in &project.edges[id] {
                if let Target::Ambiguous(ids) = &edge.target {
                    for &to in ids {
                        add_function(plan, project, to, column + 1);
                    }
                }
            }
            externals(plan, project, id, here, column + 1);
        }
        if opened.callers.contains(&key) {
            let callers = project.callers[id].clone();
            side(plan, project, &key, callers, column - 1, opened, &mut queue);
        }
        if opened.implementations.contains(&key) {
            let implementations = project.implementations(id);
            side(
                plan,
                project,
                &key,
                implementations,
                column + 1,
                opened,
                &mut queue,
            );
        }
    }
}
