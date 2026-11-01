//! Reading type names: what a declaration's type is, by name.

use super::*;

/// The name of the type `node` declares, if it declares one: a class,
/// struct, interface and the like, a Go `type_spec`, Zig's `const X =
/// struct {…}`.
pub(super) fn type_name(node: SyntaxNode, source: &str) -> Option<String> {
    let kind = node.kind();
    let declares = TYPE_KINDS.iter().any(|(k, _)| *k == kind)
        || matches!(
            kind,
            "class_definition"
                | "class_specifier"
                | "struct_specifier"
                | "type_spec"
                | "protocol_declaration"
                | "object_declaration"
        );
    if declares {
        let name = node.child_by_field_name("name")?;
        // A C `struct x;` with no body declares nothing to list.
        return Some(text(name, source).to_string());
    }
    if kind == "variable_declaration" {
        let mut cursor = node.walk();
        let children: Vec<SyntaxNode> = node.named_children(&mut cursor).collect();
        if children.iter().any(|child| {
            matches!(
                child.kind(),
                "struct_declaration" | "enum_declaration" | "union_declaration"
            )
        }) {
            let name = children.iter().find(|child| child.kind() == "identifier")?;
            return Some(text(*name, source).to_string());
        }
    }
    None
}

/// The names a declaration declares, with their types as written.
pub(super) fn declared<'t>(
    node: SyntaxNode<'t>,
    source: &str,
) -> Vec<(SyntaxNode<'t>, Option<String>)> {
    let ty = |node: SyntaxNode| {
        node.child_by_field_name("type")
            .map(|ty| one_line_type(text(ty, source)))
    };
    let mut found = Vec::new();
    let mut cursor = node.walk();
    let declarators: Vec<SyntaxNode> = node
        .children_by_field_name("declarator", &mut cursor)
        .collect();
    if !declarators.is_empty() {
        for declarator in declarators {
            if let Some(name) = declarator_name(declarator) {
                found.push((name, ty(declarator).or_else(|| ty(node))));
            }
        }
    } else if let Some(name) = node
        .child_by_field_name("name")
        .or_else(|| node.child_by_field_name("property"))
    {
        let mut cursor = node.walk();
        let names: Vec<SyntaxNode> = node.children_by_field_name("name", &mut cursor).collect();
        for name in if names.is_empty() { vec![name] } else { names } {
            if let Some(name) = declarator_name(name) {
                found.push((name, ty(node)));
            }
        }
    } else {
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            match child.kind() {
                // Python: `x = 1`, `x: int = 1`.
                "assignment" => {
                    if let Some(left) = child
                        .child_by_field_name("left")
                        .filter(|left| left.kind() == "identifier")
                    {
                        found.push((left, ty(child)));
                    }
                }
                "variable_declarator" | "var_spec" | "const_spec" | "init_declarator" => {
                    let value = child.child_by_field_name("value").map(|v| v.kind());
                    let is_function = value
                        .is_some_and(|value| is_function_like(value) || value.contains("class"));
                    if !is_function {
                        found.extend(declared(child, source));
                    }
                }
                _ => {}
            }
        }
    }
    found.retain(|(name, _)| is_name(text(*name, source)));
    found
}

/// The name a declarator declares, through pointers and arrays; `None`
/// for a function's.
pub(super) fn declarator_name(node: SyntaxNode) -> Option<SyntaxNode> {
    let kind = node.kind();
    if kind.contains("function") {
        return None;
    }
    if kind.ends_with("identifier") {
        return Some(node);
    }
    if let Some(inner) = node
        .child_by_field_name("declarator")
        .or_else(|| node.child_by_field_name("name"))
        .or_else(|| node.child_by_field_name("pattern"))
    {
        return declarator_name(inner);
    }
    let mut cursor = node.walk();
    let first = node.named_children(&mut cursor).next()?;
    first.kind().ends_with("identifier").then_some(first)
}

pub(super) fn one_line_type(written: &str) -> String {
    written
        .trim()
        .trim_start_matches(':')
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// The type names under `node`, by simple name.
pub(super) fn type_names(node: SyntaxNode, source: &str, found: &mut Vec<String>) {
    let mut stack = vec![node];
    while let Some(node) = stack.pop() {
        if matches!(node.kind(), "type_identifier" | "identifier") {
            found.push(text(node, source).to_string());
            continue;
        }
        if node.kind() == "type_arguments" {
            continue;
        }
        let mut cursor = node.walk();
        let children: Vec<_> = node.named_children(&mut cursor).collect();
        stack.extend(children.into_iter().rev());
    }
}

/// `Repo` from `: Repo<T>`, `&mut Repo`, `a.b.Repo[]`, `crate::x::Repo`.
pub fn simple_type(written: &str) -> String {
    let written = written
        .trim()
        .trim_start_matches(':')
        .trim()
        .trim_start_matches('&')
        .trim_start_matches("mut ")
        .trim_start_matches("readonly ")
        .trim();
    let base = written
        .split(['<', '[', '?', ' ', '|'])
        .next()
        .unwrap_or(written);
    base.rsplit(['.', ':']).next().unwrap_or(base).to_string()
}

/// Names declared with a type under `node`: parameters, locals, fields.
pub(crate) fn typed_names(node: SyntaxNode, source: &str, found: &mut Vec<(String, String)>) {
    let mut stack = vec![node];
    while let Some(node) = stack.pop() {
        if let Some(ty) = node.child_by_field_name("type") {
            let ty = simple_type(text(ty, source));
            let mut names = Vec::new();
            for field in ["name", "pattern"] {
                if let Some(name) = node.child_by_field_name(field) {
                    names.push(name);
                }
            }
            let mut cursor = node.walk();
            names.extend(
                node.children_by_field_name("declarator", &mut cursor)
                    .filter_map(|d| d.child_by_field_name("name").or(Some(d))),
            );
            for name in names {
                let name = text(name, source);
                if is_name(name) && !ty.is_empty() {
                    found.push((name.to_string(), ty.clone()));
                }
            }
        }
        let mut cursor = node.walk();
        stack.extend(node.named_children(&mut cursor));
    }
}

pub(super) fn is_name(text: &str) -> bool {
    !text.is_empty()
        && text
            .chars()
            .all(|c| c.is_alphanumeric() || c == '_' || c == '$')
}
