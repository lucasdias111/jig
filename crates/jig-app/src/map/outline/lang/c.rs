//! C and C++. Functions are definitions, in C++ also in classes, structs
//! and namespaces, and out-of-class ones like `Store::later`, listed under
//! `Store`. A declaration binds what it declares; a `case` runs on into
//! the next until a `break`; `goto` and `abort()` end a path; in C++, a
//! range `for` is a For Each and `try` has an output per `catch`.

use tree_sitter::Node as SyntaxNode;

use super::super::syntax::{Lister, Parts, Scope, Syntax};
use super::super::syntax::{Function, text};

pub(super) static C: Syntax = Syntax {
    separator: "::",
    functions,
    parts,
    calls: &[("call_expression", "function")],
    members: &[("field_expression", "field")],
};

pub(super) static CPP: Syntax = Syntax {
    separator: "::",
    functions,
    parts,
    calls: &[("call_expression", "function")],
    members: &[
        ("field_expression", "field"),
        ("qualified_identifier", "name"),
        ("template_function", "name"),
        ("template_method", "name"),
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
            "function_definition" => {
                let Some(name) = function_declarator(child)
                    .and_then(|declarator| declarator.child_by_field_name("declarator"))
                else {
                    continue;
                };
                let written = lister.text(name);
                // `Store::later`, defined outside its class.
                match written.rsplit_once("::") {
                    Some((owner, short)) if name.kind() == "qualified_identifier" => {
                        let inner = scope.within(lister, owner, owner.to_string());
                        found.push(inner.named(lister, child, name, short));
                    }
                    _ => found.push(scope.named(lister, child, name, written)),
                }
            }
            "namespace_definition" => {
                let Some(body) = child.child_by_field_name("body") else {
                    continue;
                };
                match child.child_by_field_name("name") {
                    Some(name) => {
                        let namespace = lister.text(name);
                        let inner =
                            scope.within(lister, namespace, format!("namespace {namespace}"));
                        collect(lister, body, &inner, found);
                    }
                    None => collect(lister, body, scope, found),
                }
            }
            "class_specifier" | "struct_specifier" => {
                let (Some(name), Some(body)) = (
                    child.child_by_field_name("name"),
                    child.child_by_field_name("body"),
                ) else {
                    continue;
                };
                let word = if child.kind() == "class_specifier" {
                    "class"
                } else {
                    "struct"
                };
                let ty = lister.text(name);
                let inner = scope.within(lister, ty, format!("{word} {ty}"));
                collect(lister, body, &inner, found);
            }
            // `template <…>`, `extern "C" { … }`, a class in a declaration.
            "template_declaration"
            | "linkage_specification"
            | "declaration_list"
            | "declaration"
            | "field_declaration" => collect(lister, child, scope, found),
            _ => {}
        }
    }
}

/// The `name(…)` part of a definition, under any `*` or `&` it returns.
fn function_declarator(definition: SyntaxNode) -> Option<SyntaxNode> {
    let mut declarator = definition.child_by_field_name("declarator")?;
    for _ in 0..8 {
        if declarator.kind() == "function_declarator" {
            return Some(declarator);
        }
        declarator = declarator
            .child_by_field_name("declarator")
            .or_else(|| declarator.named_child(0))?;
    }
    None
}

fn parts<'t>(item: SyntaxNode<'t>, _: &Function, source: &str) -> Option<Parts<'t>> {
    let declarator = function_declarator(item)?;
    let parameters = declarator
        .child_by_field_name("parameters")
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
        returns_body: false,
    })
}

/// The names a declaration declares: `x` and `p` in `int x = 1, *p;`.
fn declared(declaration: SyntaxNode) -> Vec<SyntaxNode> {
    let mut cursor = declaration.walk();
    declaration
        .children_by_field_name("declarator", &mut cursor)
        .filter_map(|declarator| {
            let mut name = match declarator.kind() {
                "init_declarator" => declarator.child_by_field_name("declarator")?,
                _ => declarator,
            };
            for _ in 0..8 {
                match name.kind() {
                    "identifier" | "structured_binding_declarator" => return Some(name),
                    "function_declarator" => return None,
                    _ => {
                        name = name
                            .child_by_field_name("declarator")
                            .or_else(|| name.named_child(0))?
                    }
                }
            }
            None
        })
        .collect()
}

#[cfg(test)]
mod tests;
