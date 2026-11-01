//! Zig. Functions are `fn`s, at the top or in a `const Store = struct
//! { … }` (`Store.method`). `const` and `var` bind, and so do the
//! `|payloads|` of loops, `if` and `catch`, for their own body; an
//! assignment is a Set; `switch` has an output per prong; `defer` and
//! `errdefer` are steps; `@panic` ends a path.

use tree_sitter::Node as SyntaxNode;

use super::super::syntax::{Function, text};
use super::super::syntax::{Lister, Parts, Scope, Syntax};

pub(super) static SYNTAX: Syntax = Syntax {
    separator: ".",
    functions,
    parts,
    calls: &[("call_expression", "function"), ("builtin_function", "")],
    members: &[("field_expression", "member")],
};

const CONTAINERS: [(&str, &str); 3] = [
    ("struct_declaration", "struct"),
    ("enum_declaration", "enum"),
    ("union_declaration", "union"),
];

fn functions(lister: &Lister, root: SyntaxNode) -> Vec<Function> {
    let mut found = Vec::new();
    collect(lister, root, &Scope::default(), &mut found);
    found
}

fn collect(lister: &Lister, node: SyntaxNode, scope: &Scope, found: &mut Vec<Function>) {
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        match child.kind() {
            "function_declaration" => {
                if let Some(name) = child.child_by_field_name("name")
                    && child.child_by_field_name("body").is_some()
                {
                    found.push(scope.function(lister, child, name));
                }
            }
            // `const Store = struct { … };`
            "variable_declaration" => {
                let mut cursor = child.walk();
                let named: Vec<SyntaxNode> = child.named_children(&mut cursor).collect();
                let name = named.iter().find(|n| n.kind() == "identifier");
                let container = named.iter().find_map(|n| {
                    CONTAINERS
                        .iter()
                        .find(|(kind, _)| *kind == n.kind())
                        .map(|(_, word)| (*n, *word))
                });
                if let (Some(name), Some((body, word))) = (name, container) {
                    let ty = lister.text(*name);
                    let inner = scope.within(lister, ty, format!("{word} {ty}"));
                    collect(lister, body, &inner, found);
                }
            }
            _ => {}
        }
    }
}

fn child_of_kind<'t>(node: SyntaxNode<'t>, kind: &str) -> Option<SyntaxNode<'t>> {
    let mut cursor = node.walk();
    node.named_children(&mut cursor)
        .find(|child| child.kind() == kind)
}

fn parts<'t>(item: SyntaxNode<'t>, _: &Function, source: &str) -> Option<Parts<'t>> {
    let parameters = child_of_kind(item, "parameters")
        .map(|list| {
            let mut cursor = list.walk();
            list.named_children(&mut cursor).collect()
        })
        .unwrap_or_default();
    Some(Parts {
        parameters,
        returns: item
            .child_by_field_name("type")
            .map(|returns| text(returns, source).to_string()),
        body: item.child_by_field_name("body"),
    })
}

#[cfg(test)]
mod tests;
