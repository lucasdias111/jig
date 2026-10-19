//! Java. Functions are methods and constructors (`Store.save`), in
//! classes, interfaces, enums and records, nested or not. A `case …:`
//! group runs on into the next until a `break`, a `case … ->` rule
//! doesn't; `try` has an output per `catch`, all on through `finally`.

use tree_sitter::Node as SyntaxNode;

use super::super::syntax::{Lister, Parts, Scope, Syntax};
use super::super::syntax::{Function, text};

pub(super) static SYNTAX: Syntax = Syntax {
    separator: ".",
    functions,
    parts,
    calls: &[
        ("method_invocation", "name"),
        ("explicit_constructor_invocation", "constructor"),
    ],
    members: &[("generic_type", ""), ("scoped_type_identifier", "")],
};

const TYPES: [(&str, &str); 4] = [
    ("class_declaration", "class"),
    ("interface_declaration", "interface"),
    ("enum_declaration", "enum"),
    ("record_declaration", "record"),
];

fn functions(lister: &Lister, root: SyntaxNode) -> Vec<Function> {
    let mut found = Vec::new();
    collect(lister, root, &Scope::default(), &mut found);
    found
}

fn collect(lister: &Lister, node: SyntaxNode, scope: &Scope, found: &mut Vec<Function>) {
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        let name = child.child_by_field_name("name");
        if let Some(&(_, word)) = TYPES.iter().find(|(kind, _)| *kind == child.kind()) {
            let (Some(name), Some(body)) = (name, child.child_by_field_name("body")) else {
                continue;
            };
            let ty = lister.text(name);
            let inner = scope.within(lister, ty, format!("{word} {ty}"));
            collect(lister, body, &inner, found);
            continue;
        }
        match child.kind() {
            "method_declaration"
            | "constructor_declaration"
            | "compact_constructor_declaration" => {
                if let Some(name) = name
                    && child.child_by_field_name("body").is_some()
                {
                    found.push(scope.function(lister, child, name));
                }
            }
            // An enum's constants come before its methods.
            "enum_body_declarations" => collect(lister, child, scope, found),
            _ => {}
        }
    }
}

fn parts<'t>(item: SyntaxNode<'t>, _: &Function, source: &str) -> Option<Parts<'t>> {
    let parameters = item
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

fn declarators(declaration: SyntaxNode) -> Vec<SyntaxNode> {
    let mut cursor = declaration.walk();
    declaration
        .children_by_field_name("declarator", &mut cursor)
        .collect()
}

#[cfg(test)]
mod tests;
