//! Python. Functions are `def`s, decorated or not, and methods
//! (`Class.method`) in classes. An assignment to a name binds it (`Set x`,
//! which data wires start from); `elif` chains on from the False; a loop's
//! `else` runs when it completes, not when it breaks; `try` has an output
//! per `except`, its `else` after the body, all on through `finally`;
//! `with` runs its body; `match` has an output per case.

use tree_sitter::Node as SyntaxNode;

use super::super::syntax::{Function, text};
use super::super::syntax::{Lister, Parts, Scope, Syntax};

pub(super) static SYNTAX: Syntax = Syntax {
    separator: ".",
    functions,
    parts,
    calls: &[("call", "function")],
    members: &[("attribute", "attribute")],
};

fn functions(lister: &Lister, root: SyntaxNode) -> Vec<Function> {
    let mut found = Vec::new();
    collect(lister, root, &Scope::default(), &mut found);
    found
}

fn collect(lister: &Lister, node: SyntaxNode, scope: &Scope, found: &mut Vec<Function>) {
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        let child = match child.kind() {
            "decorated_definition" => match child.child_by_field_name("definition") {
                Some(definition) => definition,
                None => continue,
            },
            _ => child,
        };
        let name = child.child_by_field_name("name");
        match child.kind() {
            "function_definition" => {
                if let Some(name) = name {
                    found.push(scope.function(lister, child, name));
                }
            }
            "class_definition" => {
                let (Some(name), Some(body)) = (name, child.child_by_field_name("body")) else {
                    continue;
                };
                let class = lister.text(name);
                let inner = scope.within(lister, class, format!("class {class}"));
                collect(lister, body, &inner, found);
            }
            _ => {}
        }
    }
}

fn parts<'t>(item: SyntaxNode<'t>, _: &Function, source: &str) -> Option<Parts<'t>> {
    let parameters = item.child_by_field_name("parameters")?;
    let mut cursor = parameters.walk();
    Some(Parts {
        parameters: parameters.named_children(&mut cursor).collect(),
        returns: item
            .child_by_field_name("return_type")
            .map(|returns| text(returns, source).to_string()),
        body: item.child_by_field_name("body"),
    })
}

#[cfg(test)]
mod tests;
