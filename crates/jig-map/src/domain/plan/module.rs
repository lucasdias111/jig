//! The Plan of a module, and of a file the index can't read.

use super::*;

/// The Plan of a file the index can't read: the file as one box, its
/// kind and size and that its functions aren't read, with the files
/// linking to it on the left and those it links to on the right.
pub fn unread_file(project: &Project, sources: &dyn Sources, place: &str) -> Plan {
    let mut plan = Plan::default();
    let mut centre = file_node(project, place, 0);
    centre.detail = match super::super::files::size_of(sources, place) {
        Some(size) => format!("{} · {size}", super::super::files::kind_of(place)),
        None => super::super::files::kind_of(place),
    };
    centre.note = "Its functions can't be read yet".into();
    plan.nodes.push(centre);
    plan.focus = Some(0);
    let (into, out) = super::super::files::links_of(project, sources, place);
    for (links, column, inward) in [(into, -1, true), (out, 1, false)] {
        for link in links {
            let mut node = file_node(project, &link.path, column);
            node.detail = link.how(inward);
            let ix = plan.nodes.len();
            plan.nodes.push(node);
            if inward {
                plan.wire(ix, 0, WireKind::Call);
            } else {
                plan.wire(0, ix, WireKind::Call);
            }
        }
    }
    tell_files_apart(&mut plan);
    plan
}

/// The functions of a place: a folder (`dir/`, its own files only), a
/// file (`path`) or a type in it (`path#Type`), in the order they're
/// written.
pub fn place_functions(project: &Project, place: &str) -> Vec<FnId> {
    let (path, owner) = match place.split_once('#') {
        Some((path, owner)) => (path, Some(owner)),
        None => (place, None),
    };
    let files: Vec<usize> = match path.strip_suffix('/') {
        Some(dir) => project
            .modules
            .iter()
            .find(|module| slashed(&module.dir) == dir)
            .map(|module| module.files.clone())
            .unwrap_or_default(),
        None => project
            .files
            .iter()
            .position(|file| slashed(&file.path) == path)
            .into_iter()
            .collect(),
    };
    let mut ids: Vec<FnId> = (0..project.functions.len())
        .filter(|&id| {
            let function = &project.functions[id];
            files.contains(&function.file)
                && owner.is_none_or(|owner| function.outline.owner.as_deref() == Some(owner))
        })
        .collect();
    ids.sort_by_key(|&id| {
        (
            project.functions[id].file,
            project.functions[id].range().start,
        )
    });
    ids
}

/// A module's Plan: its functions in their frames, called ones right of
/// their callers, what calls into them from elsewhere on the left, what
/// they call elsewhere on the right, and what's opened past those.
pub fn module(project: &Project, place: &str, opened: &Opened) -> Plan {
    let mut plan = Plan::default();
    let all = place_functions(project, place);
    let (shown, rest) = if opened.more.contains(&format!("{place}@module")) {
        (all.as_slice(), &[][..])
    } else {
        all.split_at(all.len().min(MAX_MODULE))
    };
    let depth = depths(project, shown);
    let deepest = depth.values().copied().max().unwrap_or(0);
    for &id in shown {
        add_function(&mut plan, project, id, depth[&id]);
    }
    if !rest.is_empty() {
        let key = format!("{place}@module");
        let mut node = Node::new(
            format!("more:{key}"),
            Item::More {
                ids: rest.to_vec(),
                key,
            },
            format!("+{} more", rest.len()),
            deepest,
        );
        node.detail = "in this module".into();
        plan.nodes.push(node);
    }
    spread_around(&mut plan, project, place, shown, deepest, opened);
    finish(&mut plan, project);
    plan
}

/// The wires, badges and hidden counts every Plan has once its boxes are
/// in.
pub(super) fn finish(plan: &mut Plan, project: &Project) {
    wire_functions(plan, project);
    badge_hubs(plan, project);
    count_hidden(plan, project);
}

/// Each function's column among `ids`: right of those calling it, as far
/// as cycles allow.
pub(super) fn depths(project: &Project, ids: &[FnId]) -> HashMap<FnId, i32> {
    let mut depth: HashMap<FnId, i32> = ids.iter().map(|&id| (id, 0)).collect();
    for _ in 0..6 {
        for &id in ids {
            for to in project.callees(id) {
                if depth.contains_key(&to) && depth[&to] <= depth[&id] && depth[&id] < 4 {
                    depth.insert(to, depth[&id] + 1);
                }
            }
        }
    }
    depth
}
