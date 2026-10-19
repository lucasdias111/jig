//! Go. Functions are `func`s and methods (`Store.Save`, listed under their
//! receiver's type). `:=`, `var` and `const` bind; `if` may start with a
//! statement of its own; `for` is a For, a For Each over a `range`, a
//! While with just a condition, or a Loop with nothing, which only a
//! `break` leaves; a `switch` case runs on into the next only with
//! `fallthrough`; `select` has an output per case; `defer` and `go` are
//! steps; `panic(…)` ends a path.

use tree_sitter::Node as SyntaxNode;

use super::super::syntax::{Lister, Parts, Scope, Syntax};
use super::super::syntax::{Function, text};

pub(super) static SYNTAX: Syntax = Syntax {
    separator: ".",
    functions,
    parts,
    calls: &[("call_expression", "function")],
    members: &[("selector_expression", "field")],
};

fn functions(lister: &Lister, root: SyntaxNode) -> Vec<Function> {
    let mut found = Vec::new();
    let top = Scope::default();
    let mut cursor = root.walk();
    for child in root.named_children(&mut cursor) {
        let Some(name) = child.child_by_field_name("name") else {
            continue;
        };
        match child.kind() {
            "function_declaration" => found.push(top.function(lister, child, name)),
            "method_declaration" => {
                let receiver = child
                    .child_by_field_name("receiver")
                    .and_then(|receiver| receiver.named_child(0))
                    .and_then(|parameter| parameter.child_by_field_name("type"))
                    .map(|ty| receiver_type(lister.text(ty)));
                let scope = match receiver {
                    Some(ty) => top.within(lister, &ty, ty.clone()),
                    None => top.clone(),
                };
                found.push(scope.function(lister, child, name));
            }
            _ => {}
        }
    }
    found
}

/// `Store` from `*Store` or `Store[T]`.
fn receiver_type(ty: &str) -> String {
    let ty = ty.trim_start_matches('*');
    ty.split('[').next().unwrap_or(ty).trim().to_string()
}

fn parts<'t>(item: SyntaxNode<'t>, _: &Function, source: &str) -> Option<Parts<'t>> {
    let mut parameters = Vec::new();
    for list in ["receiver", "parameters"] {
        if let Some(list) = item.child_by_field_name(list) {
            let mut cursor = list.walk();
            parameters.extend(list.named_children(&mut cursor));
        }
    }
    Some(Parts {
        parameters,
        returns: item
            .child_by_field_name("result")
            .map(|result| text(result, source).to_string()),
        body: item.child_by_field_name("body"),
        returns_body: false,
    })
}

#[cfg(test)]
mod tests;
