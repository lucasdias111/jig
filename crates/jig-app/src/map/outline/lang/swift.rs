//! Swift. Functions are `func`s and `init`s, in classes, structs, enums,
//! actors and extensions (`Store.save`). `let` and `var` bind; `guard`
//! is a Branch whose False must leave; a `switch` case runs on into the
//! next only with `fallthrough`; `do` has an output per `catch`; `defer`
//! is a step; `fatalError()` ends a path.

use tree_sitter::Node as SyntaxNode;

use super::super::syntax::{Lister, Parts, Scope, Syntax};
use super::super::syntax::{Function, text};

pub(super) static SYNTAX: Syntax = Syntax {
    separator: ".",
    functions,
    parts,
    calls: &[("call_expression", "")],
    members: &[
        ("navigation_expression", "suffix"),
        ("navigation_suffix", "suffix"),
    ],
};

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
            "init_declaration" => {
                if child.child_by_field_name("body").is_some() {
                    found.push(scope.named(lister, child, child, "init"));
                }
            }
            "class_declaration" => {
                let (Some(name), Some(body)) = (
                    child.child_by_field_name("name"),
                    child.child_by_field_name("body"),
                ) else {
                    continue;
                };
                let word = child.child(0).map_or("class", |word| lister.text(word));
                let ty = lister.text(name);
                let inner = scope.within(lister, ty, format!("{word} {ty}"));
                collect(lister, body, &inner, found);
            }
            _ => {}
        }
    }
}

fn parts<'t>(item: SyntaxNode<'t>, _: &Function, source: &str) -> Option<Parts<'t>> {
    let mut cursor = item.walk();
    let parameters = item
        .named_children(&mut cursor)
        .filter(|child| child.kind() == "parameter")
        .collect();
    // The first `name` is the function's, the next what it returns.
    let mut cursor = item.walk();
    let returns = item
        .children_by_field_name("name", &mut cursor)
        .nth(1)
        .map(|returns| text(returns, source).to_string());
    let body = item
        .child_by_field_name("body")
        .and_then(|body| child_of_kind(body, "statements"));
    Some(Parts {
        parameters,
        returns,
        body,
        returns_body: false,
    })
}

fn child_of_kind<'t>(node: SyntaxNode<'t>, kind: &str) -> Option<SyntaxNode<'t>> {
    let mut cursor = node.walk();
    node.named_children(&mut cursor)
        .find(|child| child.kind() == kind)
}

#[cfg(test)]
mod tests;
