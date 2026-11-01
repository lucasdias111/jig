//! Plan's boxes: a function's, a file's, a type's frame.

use super::*;

/// Files with the same name (`+page.svelte` in each route) titled with as
/// many of their folders as tell them apart: `hold/[id]/+page.svelte`.
pub(super) fn tell_files_apart(plan: &mut Plan) {
    let paths: Vec<(usize, String)> = plan
        .nodes
        .iter()
        .enumerate()
        .filter_map(|(ix, node)| match &node.item {
            Item::File { path, .. } => Some((ix, path.clone())),
            _ => None,
        })
        .collect();
    let tail = |path: &str, parts: usize| {
        let all: Vec<&str> = path.split('/').collect();
        all[all.len().saturating_sub(parts)..].join("/")
    };
    for (ix, path) in &paths {
        let parts = (1..=path.split('/').count())
            .find(|&parts| {
                let mine = tail(path, parts);
                paths
                    .iter()
                    .all(|(other, theirs)| other == ix || tail(theirs, parts) != mine)
            })
            .unwrap_or(1);
        plan.nodes[*ix].title = tail(path, parts);
    }
}

/// File `place` as a box in a Plan, in column `column`.
pub(super) fn file_node(project: &Project, place: &str, column: i32) -> Node {
    let read = project
        .files
        .iter()
        .any(|file| slashed(&file.path) == place);
    let title = place.rsplit('/').next().unwrap_or(place).to_string();
    Node::new(
        format!("file:{place}"),
        Item::File {
            path: place.to_string(),
            read,
        },
        title,
        column,
    )
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
pub(super) fn compact_type(ty: &str) -> String {
    let ty = ty.split_whitespace().collect::<Vec<_>>().join(" ");
    if ty.chars().count() <= 18 {
        return ty;
    }
    simple_type(&ty)
}

/// The function node for `id`.
pub(super) fn function_node(project: &Project, id: FnId, column: i32) -> Node {
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

pub(super) fn add_function(plan: &mut Plan, project: &Project, id: FnId, column: i32) -> usize {
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
