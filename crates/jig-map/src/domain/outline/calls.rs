//! A function's contract and the calls it makes.

use super::*;

/// A function's parameters, what it returns, and its body.
pub(super) fn contract<'t>(
    syntax: Option<&Syntax>,
    item: SyntaxNode<'t>,
    function: &Function,
    source: &str,
) -> (Vec<Parameter>, Option<String>, Option<SyntaxNode<'t>>) {
    let (nodes, returns, body) = match syntax {
        Some(syntax) => match (syntax.parts)(item, function, source) {
            Some(parts) => (parts.parameters, parts.returns, parts.body),
            None => (Vec::new(), None, None),
        },
        None => {
            let nodes = item
                .child_by_field_name("parameters")
                .map(|list| {
                    let mut cursor = list.walk();
                    list.named_children(&mut cursor)
                        .filter(|p| !p.is_extra())
                        .collect()
                })
                .unwrap_or_default();
            let returns = item
                .child_by_field_name("return_type")
                .map(|r| text(r, source).to_string());
            (nodes, returns, item.child_by_field_name("body"))
        }
    };
    let parameters = nodes
        .into_iter()
        .filter(|node| !node.is_extra())
        .map(|node| parameter(node, source))
        .collect();
    let returns = returns
        .map(|r| r.trim().trim_start_matches(':').trim().to_string())
        .filter(|r| !r.is_empty() && r != "void" && r != "()");
    (parameters, returns, body)
}

pub(super) fn parameter(node: SyntaxNode, source: &str) -> Parameter {
    let name = node
        .child_by_field_name("name")
        .or_else(|| node.child_by_field_name("pattern"))
        .map(|name| text(name, source).to_string());
    let ty = node.child_by_field_name("type").map(|ty| {
        text(ty, source)
            .trim()
            .trim_start_matches(':')
            .trim()
            .to_string()
    });
    match name {
        Some(name) => Parameter { name, ty },
        None => Parameter {
            name: text(node, source)
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" "),
            ty: None,
        },
    }
}

/// The calls under `item`, and the functions it hands on.
pub(super) fn calls(
    syntax: Option<&Syntax>,
    item: SyntaxNode,
    source: &str,
    parameters: &[&str],
) -> Vec<Call> {
    let mut found = Vec::new();
    let mut stack = vec![item];
    while let Some(node) = stack.pop() {
        let name = match syntax {
            Some(syntax) => callee_name(syntax, node),
            None if node.kind() == "call_expression" => node
                .child_by_field_name("function")
                .and_then(callee_name_node),
            None => None,
        };
        if let Some(name) = name {
            let lead = source
                .get(node.start_byte()..name.start_byte())
                .unwrap_or("");
            let qualifier = qualifier(lead);
            let called = text(name, source).to_string();
            found.push(Call {
                through_parameter: qualifier.is_none() && parameters.contains(&called.as_str()),
                name: called,
                qualifier,
                at: name.start_byte(),
                bound: bound(node, source),
                arguments: arguments(node, source),
                reference: false,
            });
            // What it's handed that may be a function.
            if let Some(list) = node.child_by_field_name("arguments") {
                let mut cursor = list.walk();
                for argument in list.named_children(&mut cursor) {
                    if let Some(reference) = handed(argument, source) {
                        found.push(reference);
                    }
                }
            }
        } else if node.kind() == "method_reference" {
            // Java: `this::handle`, `Repo::save`.
            let written = text(node, source);
            if let Some((on, method)) = written.rsplit_once("::") {
                found.push(Call {
                    name: method.trim().to_string(),
                    qualifier: qualifier(&format!("{on}.")),
                    at: node.start_byte() + written.len() - method.len(),
                    reference: true,
                    ..Call::default()
                });
            }
        }
        let mut cursor = node.walk();
        let children: Vec<_> = node.named_children(&mut cursor).collect();
        stack.extend(children.into_iter().rev());
    }
    found.sort_by_key(|call| call.at);
    found
}

/// What a call is made on, from the text before its name: `this.repo.`
/// is `repo`; `this.` none; `a().` an expression (`""`).
pub(super) fn qualifier(lead: &str) -> Option<String> {
    let lead = lead.trim_end_matches(['.', ':', '-', '>', '?', ' ', '\n', '\t']);
    if lead.is_empty() {
        return None;
    }
    let last = lead
        .rsplit(['.', ':', ' ', '(', '\n'])
        .next()
        .unwrap_or(lead);
    match last {
        "this" | "self" | "Self" | "super" => None,
        _ if lead.ends_with(')') || lead.ends_with(']') || !is_name(last) => Some(String::new()),
        _ => Some(last.to_string()),
    }
}

/// The name a call's result is bound to, if it's bound to one.
pub(super) fn bound(call: SyntaxNode, source: &str) -> Option<String> {
    let mut node = call;
    for _ in 0..4 {
        let parent = node.parent()?;
        let value = parent
            .child_by_field_name("value")
            .or_else(|| parent.child_by_field_name("right"));
        if value.is_some_and(|value| value.byte_range().contains(&call.start_byte())) {
            let name = parent
                .child_by_field_name("name")
                .or_else(|| parent.child_by_field_name("pattern"))
                .or_else(|| parent.child_by_field_name("left"))?;
            let name = text(name, source).trim_start_matches("mut ").to_string();
            return is_name(&name).then_some(name);
        }
        if parent.kind().ends_with("statement") || parent.kind().ends_with("block") {
            return None;
        }
        node = parent;
    }
    None
}

/// The names a call is handed.
pub(super) fn arguments(call: SyntaxNode, source: &str) -> Vec<String> {
    let Some(list) = call.child_by_field_name("arguments") else {
        return Vec::new();
    };
    let mut names: Vec<String> = Vec::new();
    let mut stack = vec![list];
    while let Some(node) = stack.pop() {
        if node.kind() == "identifier" {
            let name = text(node, source).to_string();
            if !names.contains(&name) {
                names.push(name);
            }
            continue;
        }
        let mut cursor = node.walk();
        let children: Vec<_> = node.named_children(&mut cursor).collect();
        stack.extend(children.into_iter().rev());
    }
    names
}

/// An argument that may be a function handed on: a name, or a member
/// (`this.handle`).
pub(super) fn handed(argument: SyntaxNode, source: &str) -> Option<Call> {
    if !matches!(
        argument.kind(),
        "identifier" | "member_expression" | "field_expression" | "scoped_identifier"
    ) {
        return None;
    }
    let written = text(argument, source);
    let (lead, name) = match written.rfind(['.', ':']) {
        Some(at) => (&written[..=at], &written[at + 1..]),
        None => ("", written),
    };
    is_name(name).then(|| Call {
        name: name.to_string(),
        qualifier: qualifier(lead),
        at: argument.start_byte() + lead.len(),
        reference: true,
        ..Call::default()
    })
}
