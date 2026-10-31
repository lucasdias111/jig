//! JavaScript and TypeScript (and TSX). Functions are declarations, `const`
//! and `let` bindings of arrow functions and function expressions, and
//! class methods (`Class.method`), in classes and namespaces. `switch` runs
//! on into the next case until a `break`; `try` has Body and Catch, both on
//! through `finally`; an arrow function's expression is its Return.

use tree_sitter::Node as SyntaxNode;

use super::super::syntax::{Function, text};
use super::super::syntax::{Lister, Parts, Scope, Syntax};

pub(super) static SYNTAX: Syntax = Syntax {
    separator: ".",
    functions,
    parts,
    calls: &[("call_expression", "function")],
    members: &[
        ("member_expression", "property"),
        ("non_null_expression", ""),
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
        let name = child.child_by_field_name("name");
        match child.kind() {
            "export_statement" | "ambient_declaration" | "expression_statement" => {
                collect(lister, child, scope, found)
            }
            "function_declaration" | "generator_function_declaration" => {
                if let Some(name) = name {
                    found.push(scope.function(lister, child, name));
                }
            }
            "lexical_declaration" | "variable_declaration" => {
                let declarators = declarators(child);
                for declarator in &declarators {
                    let (Some(name), Some(value)) = (
                        declarator.child_by_field_name("name"),
                        declarator.child_by_field_name("value"),
                    ) else {
                        continue;
                    };
                    if name.kind() == "identifier" && is_function(value.kind()) {
                        // One binding: the whole statement, `const` and all.
                        let item = if declarators.len() == 1 {
                            child
                        } else {
                            *declarator
                        };
                        found.push(scope.function(lister, item, name));
                    }
                }
            }
            "class_declaration" | "abstract_class_declaration" => {
                let (Some(name), Some(body)) = (name, child.child_by_field_name("body")) else {
                    continue;
                };
                let class = lister.text(name);
                let inner = scope.within(lister, class, format!("class {class}"));
                let mut cursor = body.walk();
                for member in body.named_children(&mut cursor) {
                    let field = |field: &str| member.child_by_field_name(field);
                    let member_name = field("name").or_else(|| field("property"));
                    let has_body = match member.kind() {
                        "method_definition" => field("body").is_some(),
                        "public_field_definition" | "field_definition" => {
                            field("value").is_some_and(|value| is_function(value.kind()))
                        }
                        _ => false,
                    };
                    if has_body && let Some(member_name) = member_name {
                        found.push(inner.function(lister, member, member_name));
                    }
                }
            }
            "module" | "internal_module" => {
                let (Some(name), Some(body)) = (name, child.child_by_field_name("body")) else {
                    continue;
                };
                let namespace = lister.text(name);
                let inner = scope.within(lister, namespace, format!("namespace {namespace}"));
                collect(lister, body, &inner, found);
            }
            _ => {}
        }
    }
}

fn declarators(declaration: SyntaxNode) -> Vec<SyntaxNode> {
    let mut cursor = declaration.walk();
    declaration
        .named_children(&mut cursor)
        .filter(|d| d.kind() == "variable_declarator")
        .collect()
}

fn is_function(kind: &str) -> bool {
    matches!(
        kind,
        "arrow_function"
            | "function_expression"
            | "function"
            | "generator_function"
            | "function_declaration"
            | "generator_function_declaration"
            | "method_definition"
    )
}

/// The function itself in `item`: the item, or the function a `const` or
/// a class field holds.
fn parts<'t>(item: SyntaxNode<'t>, function: &Function, source: &str) -> Option<Parts<'t>> {
    let node = if is_function(item.kind()) {
        item
    } else {
        let mut stack = vec![item];
        let mut found = None;
        while let Some(node) = stack.pop() {
            let value = node.child_by_field_name("value");
            let name = node.child_by_field_name("name");
            if let Some(value) = value
                && is_function(value.kind())
                && name.is_none_or(|name| name.start_byte() == function.name_at)
            {
                found = Some(value);
                break;
            }
            let mut cursor = node.walk();
            stack.extend(node.named_children(&mut cursor));
        }
        found?
    };
    let body = node.child_by_field_name("body");
    Some(Parts {
        parameters: parameters(node),
        returns: node
            .child_by_field_name("return_type")
            .map(|returns| text(returns, source).trim_start_matches(':').to_string()),
        body,
    })
}

/// `(a, b)`'s parameters, or `a =>`'s.
fn parameters(function: SyntaxNode) -> Vec<SyntaxNode> {
    if let Some(parameters) = function.child_by_field_name("parameters") {
        let mut cursor = parameters.walk();
        return parameters.named_children(&mut cursor).collect();
    }
    function
        .child_by_field_name("parameter")
        .into_iter()
        .collect()
}

#[cfg(test)]
mod tests;
