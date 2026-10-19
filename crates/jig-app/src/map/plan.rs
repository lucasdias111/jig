//! The Plan level: a neighbourhood around a focus, never the whole
//! project. A function's is its callers on the left and its callees on
//! the right, one hop out, more opened on demand; a module's (a folder, a
//! file or a type) is its functions framed by type, called ones right of
//! their callers, with the calls in from elsewhere on the left and out to
//! elsewhere on the right. Functions sit in frames for their type or
//! file.
//!
//! What would drown the picture is folded: past 8 callers or callees on a
//! side the rest are one "+N more" box, a side's tests are one box, calls
//! into a package outside the project are one box per package, and
//! functions called from all over (hubs) are badges on their callers.

use std::collections::{HashMap, HashSet};

use super::index::{FnId, Func, Project, Symbol, Target};
use super::outline::simple_type;

/// Callers or callees shown on a side before the rest fold away.
pub const MAX_SIDE: usize = 8;
/// A module's own functions shown before the rest fold away.
pub const MAX_MODULE: usize = 80;

/// What a Plan is about.
#[derive(Clone, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum Focus {
    /// A function, coming from the code or another function.
    Function(Symbol),
    /// A module, coming from Site: a folder (`dir/`), a file (`path`) or a
    /// type in one (`path#Type`).
    Module(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Item {
    Function(FnId),
    /// A function a Plan was about that's no longer found.
    Gone(Symbol),
    /// Calls into a package outside the project.
    External(String),
    /// Functions folded away: on a side, or the rest of a module.
    More {
        ids: Vec<FnId>,
        key: String,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Node {
    /// What keeps it across rebuilds, and in saved places.
    pub key: String,
    pub item: Item,
    pub frame: Option<usize>,
    /// Left to right: callers before callees.
    pub column: i32,
    pub title: String,
    /// Below the title: a compact signature, a package's call count.
    pub detail: String,
    /// Hubs it calls, folded: `log ×3`.
    pub badges: Vec<String>,
    /// Callers and callees not drawn, for its expand buttons.
    pub hidden_callers: usize,
    pub hidden_callees: usize,
    /// For an interface's method: its implementations, not drawn yet.
    pub implementations: usize,
    /// It calls a function it was handed, whatever that is.
    pub unknown_target: bool,
}

impl Node {
    fn new(key: String, item: Item, title: String, column: i32) -> Self {
        Node {
            key,
            item,
            frame: None,
            column,
            title,
            detail: String::new(),
            badges: Vec::new(),
            hidden_callers: 0,
            hidden_callees: 0,
            implementations: 0,
            unknown_target: false,
        }
    }

    pub fn function(&self) -> Option<FnId> {
        match self.item {
            Item::Function(id) => Some(id),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WireKind {
    /// A calls B.
    Call,
    /// A calls something named like B, one of a few.
    Ambiguous,
    /// A hands B on, to be called later.
    Callback,
}

impl WireKind {
    /// Drawn dashed: a guess, or a function handed on.
    pub fn dashed(&self) -> bool {
        matches!(self, WireKind::Ambiguous | WireKind::Callback)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Wire {
    pub from: usize,
    pub to: usize,
    pub kind: WireKind,
}

/// A type or file the nodes in it sit inside.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frame {
    pub key: String,
    pub title: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Plan {
    pub nodes: Vec<Node>,
    pub wires: Vec<Wire>,
    pub frames: Vec<Frame>,
    /// The function a function's Plan is about.
    pub focus: Option<usize>,
}

impl Plan {
    pub fn index_of(&self, key: &str) -> Option<usize> {
        self.nodes.iter().position(|node| node.key == key)
    }

    fn frame(&mut self, key: String, title: String) -> usize {
        match self.frames.iter().position(|frame| frame.key == key) {
            Some(ix) => ix,
            None => {
                self.frames.push(Frame { key, title });
                self.frames.len() - 1
            }
        }
    }

    fn wire(&mut self, from: usize, to: usize, kind: WireKind) {
        let wire = Wire { from, to, kind };
        if from != to && !self.wires.contains(&wire) {
            self.wires.push(wire);
        }
    }
}

/// What's opened in a Plan beyond its first hop.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Opened {
    /// Functions whose callers, or callees, are shown too.
    pub callers: HashSet<String>,
    pub callees: HashSet<String>,
    /// Interface methods whose implementations are shown.
    pub implementations: HashSet<String>,
    /// "+N more" groups opened.
    pub more: HashSet<String>,
}

/// The Plan of `focus`, with what's opened past its first hop.
pub fn plan(project: &Project, focus: &Focus, opened: &Opened) -> Plan {
    match focus {
        Focus::Function(symbol) => match project.find(symbol) {
            Some(id) => neighbourhood(project, id, opened),
            None => gone(symbol),
        },
        Focus::Module(place) => module(project, place, opened),
    }
}

/// The key a function keeps on every level.
pub fn function_key(project: &Project, id: FnId) -> String {
    let symbol = project.symbol(id);
    format!(
        "fn:{}#{}.{}/{}",
        symbol.file,
        symbol.owner.unwrap_or_default(),
        symbol.name,
        symbol.arity
    )
}

/// A function's compact signature, typed where its code says:
/// `(title: String, id: long) → Bug`. A method's own receiver is left
/// out.
pub fn signature(function: &Func) -> String {
    let parameters: Vec<String> = function
        .outline
        .parameters
        .iter()
        .filter(|p| !matches!(p.name.as_str(), "self" | "&self" | "&mut self" | "this"))
        .map(|p| match &p.ty {
            Some(ty) => format!("{}: {}", p.name, compact_type(ty)),
            None => p.name.clone(),
        })
        .collect();
    let mut signature = format!("({})", parameters.join(", "));
    if let Some(returns) = &function.outline.returns {
        signature.push_str(" → ");
        signature.push_str(&compact_type(returns));
    }
    signature
}

/// A type as short as it reads: `&mut Context<Self>` as `Context`,
/// `Vec<FnId>` as it is.
fn compact_type(ty: &str) -> String {
    let ty = ty.split_whitespace().collect::<Vec<_>>().join(" ");
    if ty.chars().count() <= 18 {
        return ty;
    }
    simple_type(&ty)
}

/// The function node for `id`.
fn function_node(project: &Project, id: FnId, column: i32) -> Node {
    let function = &project.functions[id];
    let mut node = Node::new(
        function_key(project, id),
        Item::Function(id),
        function.title(),
        column,
    );
    node.detail = signature(function);
    node.unknown_target = project.edges[id]
        .iter()
        .any(|edge| edge.target == Target::Unknown);
    if function.outline.is_abstract {
        node.detail = format!("{} · interface", node.detail);
    }
    node
}

/// The frame a function sits in: its type, or its file. Its key is the
/// function's place: `path#Type`, or `path`.
pub fn frame_of(project: &Project, id: FnId) -> (String, String) {
    let function = &project.functions[id];
    let file = &project.files[function.file];
    let path = file.path.to_string_lossy().replace('\\', "/");
    match &function.outline.owner {
        Some(owner) => {
            let kind = file
                .types
                .iter()
                .find(|ty| &ty.name == owner)
                .map_or("type", |ty| ty.kind);
            (format!("{path}#{owner}"), format!("{kind} {owner}"))
        }
        None => (
            path.clone(),
            file.path
                .file_name()
                .map_or(path, |name| name.to_string_lossy().into_owned()),
        ),
    }
}

fn add_function(plan: &mut Plan, project: &Project, id: FnId, column: i32) -> usize {
    let key = function_key(project, id);
    if let Some(ix) = plan.index_of(&key) {
        return ix;
    }
    let mut node = function_node(project, id, column);
    let (frame_key, title) = frame_of(project, id);
    node.frame = Some(plan.frame(frame_key, title));
    plan.nodes.push(node);
    plan.nodes.len() - 1
}

/// Function `focus` with its callers and callees, and what's opened past
/// them.
pub fn neighbourhood(project: &Project, focus: FnId, opened: &Opened) -> Plan {
    let mut plan = Plan::default();
    let at = add_function(&mut plan, project, focus, 0);
    plan.focus = Some(at);
    let key = function_key(project, focus);
    spread(&mut plan, project, &key, &[focus], opened);
    finish(&mut plan, project);
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

/// A path as the Map's keys write it, with forward slashes.
pub fn slashed(path: &std::path::Path) -> String {
    path.to_string_lossy().replace('\\', "/")
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
fn finish(plan: &mut Plan, project: &Project) {
    wire_functions(plan, project);
    badge_hubs(plan, project);
    count_hidden(plan, project);
}

/// Each function's column among `ids`: right of those calling it, as far
/// as cycles allow.
fn depths(project: &Project, ids: &[FnId]) -> HashMap<FnId, i32> {
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

/// Around a module's functions, in columns 0 to `deepest`: its callers
/// from elsewhere left of them, what they call elsewhere right of them.
fn spread_around(
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
fn spread(plan: &mut Plan, project: &Project, of: &str, centre: &[FnId], opened: &Opened) {
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
fn open_hops(
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

/// Functions on one side of `of`, past `MAX_SIDE` folded into a group.
fn side(
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
fn externals(plan: &mut Plan, project: &Project, id: FnId, here: usize, column: i32) {
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
fn wire_functions(plan: &mut Plan, project: &Project) {
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
fn badge_hubs(plan: &mut Plan, project: &Project) {
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
fn count_hidden(plan: &mut Plan, project: &Project) {
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

/// What a Plan about function `symbol` shows once it's no longer found.
pub fn gone(symbol: &Symbol) -> Plan {
    let mut node = Node::new(
        format!("gone:{}#{}", symbol.file, symbol.title()),
        Item::Gone(symbol.clone()),
        symbol.title(),
        0,
    );
    node.detail = "gone: no longer found".into();
    Plan {
        nodes: vec![node],
        ..Plan::default()
    }
}

#[cfg(test)]
mod tests {
    use super::super::index::tests::{fixture, id};
    use super::*;

    fn titles(plan: &Plan) -> Vec<&str> {
        plan.nodes.iter().map(|n| n.title.as_str()).collect()
    }

    fn wired(plan: &Plan, from: &str, to: &str) -> Option<WireKind> {
        let from = plan.nodes.iter().position(|n| n.title == from)?;
        let to = plan.nodes.iter().position(|n| n.title == to)?;
        plan.wires
            .iter()
            .find(|w| w.from == from && w.to == to)
            .map(|w| w.kind.clone())
    }

    #[test]
    fn a_function_plan_is_one_hop_with_externals_and_interfaces() {
        let dir = fixture();
        let project = Project::read(dir.path());
        let create = id(&project, "BugService.create");
        let plan = neighbourhood(&project, create, &Opened::default());
        assert_eq!(
            titles(&plan),
            [
                "BugService.create",
                "UserService.find",
                "BugService.save",
                "Notifier.send",
                "org.slf4j"
            ]
        );
        assert_eq!(
            wired(&plan, "BugService.create", "UserService.find"),
            Some(WireKind::Call)
        );
        assert_eq!(
            wired(&plan, "BugService.create", "org.slf4j"),
            Some(WireKind::Call)
        );
        let focus = &plan.nodes[plan.focus.unwrap()];
        assert_eq!(focus.detail, "(title: String, userId: long) → Bug");
        let send = plan
            .nodes
            .iter()
            .find(|n| n.title == "Notifier.send")
            .unwrap();
        assert_eq!(send.implementations, 1, "one implementation to open");
        let find = plan
            .nodes
            .iter()
            .find(|n| n.title == "UserService.find")
            .unwrap();
        assert_eq!(find.hidden_callees, 1, "lookup, a hop further");
        let frames: Vec<&str> = plan.frames.iter().map(|f| f.title.as_str()).collect();
        assert_eq!(
            frames,
            [
                "class BugService",
                "class UserService",
                "interface Notifier"
            ]
        );
    }

    #[test]
    fn opening_a_box_adds_its_next_hop() {
        let dir = fixture();
        let project = Project::read(dir.path());
        let create = id(&project, "BugService.create");
        let mut opened = Opened::default();
        opened
            .callees
            .insert(function_key(&project, id(&project, "UserService.find")));
        opened
            .implementations
            .insert(function_key(&project, id(&project, "Notifier.send")));
        let plan = neighbourhood(&project, create, &opened);
        assert!(titles(&plan).contains(&"UserService.lookup"));
        assert!(titles(&plan).contains(&"MailNotifier.send"));
        assert_eq!(
            wired(&plan, "Notifier.send", "MailNotifier.send"),
            Some(WireKind::Ambiguous)
        );
    }

    #[test]
    fn callbacks_of_parameters_are_unknown_targets() {
        let dir = fixture();
        let project = Project::read(dir.path());
        let plan = neighbourhood(&project, id(&project, "join"), &Opened::default());
        let join = &plan.nodes[plan.focus.unwrap()];
        assert!(join.unknown_target);
        assert!(titles(&plan).contains(&"mediasoup"));
        assert_eq!(
            join.detail,
            "(id: string, done: () => void) → Promise<void>"
        );
    }

    #[test]
    fn a_module_plan_frames_its_functions_with_calls_in_and_out_at_the_edges() {
        let dir = fixture();
        let project = Project::read(dir.path());
        let bugs = module(&project, "server/src/org/demo/bugs/", &Opened::default());
        let column = |plan: &Plan, title: &str| {
            plan.nodes
                .iter()
                .find(|n| n.title == title)
                .map(|n| n.column)
                .unwrap_or_else(|| panic!("no {title} in {:?}", titles(plan)))
        };
        // Its own, framed by type, called ones right of their callers.
        assert!(column(&bugs, "BugService.save") > column(&bugs, "BugService.create"));
        let frames: Vec<&str> = bugs.frames.iter().map(|f| f.title.as_str()).collect();
        assert!(frames.contains(&"class BugService") && frames.contains(&"class MailNotifier"));
        // What it calls elsewhere, on the right.
        assert!(column(&bugs, "UserService.find") > column(&bugs, "BugService.save"));
        assert!(column(&bugs, "org.slf4j") > column(&bugs, "BugService.create"));
        // And a file's, with its callers from elsewhere on the left.
        let users = module(
            &project,
            "server/src/org/demo/users/UserService.java",
            &Opened::default(),
        );
        assert_eq!(column(&users, "BugService.create"), -1);
    }

    #[test]
    fn a_side_past_eight_folds_into_more() {
        let dir = tempfile::tempdir().unwrap();
        let mut source = String::from("fn hub() {}\n");
        for n in 0..11 {
            source.push_str(&format!("fn caller{n}() {{ hub(); }}\n"));
        }
        std::fs::write(dir.path().join("lib.rs"), source).unwrap();
        let project = Project::read(dir.path());
        let plan = neighbourhood(&project, id(&project, "hub"), &Opened::default());
        let callers = plan.nodes.iter().filter(|n| n.column == -1).count();
        assert_eq!(callers, MAX_SIDE, "seven callers and a +N more");
        assert!(titles(&plan).contains(&"+4 more"));
        let mut opened = Opened::default();
        opened.more.insert(format!(
            "{}@-1",
            function_key(&project, id(&project, "hub"))
        ));
        let all = neighbourhood(&project, id(&project, "hub"), &opened);
        assert_eq!(all.nodes.iter().filter(|n| n.column == -1).count(), 11);
    }
}
