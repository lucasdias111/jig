//! Swift. Functions are `func`s and `init`s, in classes, structs, enums,
//! actors and extensions (`Store.save`). `let` and `var` bind; `guard`
//! is a Branch whose False must leave; a `switch` case runs on into the
//! next only with `fallthrough`; `do` has an output per `catch`; `defer`
//! is a step; `fatalError()` ends a path.

use tree_sitter::Node as SyntaxNode;

use crate::blueprint::flow::data::{Binds, Names};
use crate::blueprint::flow::{Case, Exits, Flow, Handler, Lister, Parts, Scope, Syntax};
use crate::blueprint::{Function, one_line, text};

pub(in crate::blueprint) static SYNTAX: Syntax = Syntax {
    separator: ".",
    functions,
    parts,
    statement,
    calls: &[("call_expression", "")],
    members: &[
        ("navigation_expression", "suffix"),
        ("navigation_suffix", "suffix"),
    ],
    news: &[],
    sets: &[("assignment", "target")],
    transparent: &["try_expression", "await_expression"],
    panics: &["fatalError", "preconditionFailure"],
    data: Names {
        names: &["simple_identifier"],
        scopes: &["statements"],
        skip: &[
            "user_type",
            "type_annotation",
            "type_identifier",
            "array_type",
            "optional_type",
            "function_type",
            "tuple_type",
            "dictionary_type",
            "value_binding_pattern",
            "navigation_suffix",
            "statement_label",
        ],
        not_variables: &[("parameter", "external_name"), ("value_argument", "name")],
        defaults: &[],
        binds,
    },
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

/// A statement's children: what comes before its `else`, and the one
/// after it.
fn around_else(statement: SyntaxNode) -> (Vec<SyntaxNode>, Option<SyntaxNode>) {
    let mut cursor = statement.walk();
    let children: Vec<SyntaxNode> = statement.children(&mut cursor).collect();
    match children.iter().position(|child| child.kind() == "else") {
        Some(at) => (
            children[..at].to_vec(),
            children[at + 1..]
                .iter()
                .find(|child| child.is_named())
                .copied(),
        ),
        None => (children, None),
    }
}

fn statement(flow: &mut Flow, statement: SyntaxNode, exits: Exits) -> Exits {
    let field = |name: &str| statement.child_by_field_name(name);
    match statement.kind() {
        "statements" => flow.block(statement, exits),
        "property_declaration" => {
            let keyword = child_of_kind(statement, "value_binding_pattern")
                .map_or("let", |keyword| flow.text(keyword));
            let keyword = if keyword == "var" { "Var" } else { "Let" };
            let mut cursor = statement.walk();
            let names: Vec<String> = statement
                .children_by_field_name("name", &mut cursor)
                .map(|name| one_line(flow.text(name)))
                .collect();
            flow.bind(
                statement,
                format!("{keyword} {}", names.join(", ")),
                field("value"),
                exits,
            )
        }
        "if_statement" | "guard_statement" => {
            let (before, otherwise) = around_else(statement);
            let then = before
                .iter()
                .find(|child| child.kind() == "statements")
                .copied();
            let word = if statement.kind() == "if_statement" {
                "if"
            } else {
                "guard"
            };
            let end = then
                .map(|then| then.start_byte())
                .or_else(|| before.last().map(|last| last.end_byte()))
                .unwrap_or(statement.end_byte());
            let condition = flow.head(statement.start_byte()..end, &[word]);
            flow.branch(statement, &condition, then, otherwise, exits)
        }
        "for_statement" => {
            let item = field("item").map_or("", |item| flow.text(item));
            let collection = field("collection").map_or("", |c| flow.text(c));
            let head = format!("{item} in {collection}");
            let body = child_of_kind(statement, "statements");
            flow.loop_on(statement, "For Each", &head, body, true, exits)
        }
        "while_statement" | "repeat_while_statement" => {
            let condition = flow.condition(field("condition"));
            let title = if statement.kind() == "while_statement" {
                "While"
            } else {
                "Repeat While"
            };
            let body = child_of_kind(statement, "statements");
            flow.loop_on(statement, title, &condition, body, true, exits)
        }
        "switch_statement" => {
            let mut cursor = statement.walk();
            let entries: Vec<SyntaxNode> = statement
                .named_children(&mut cursor)
                .filter(|child| child.kind() == "switch_entry")
                .collect();
            let cases: Vec<Case> = entries
                .iter()
                .map(|entry| {
                    let mut cursor = entry.walk();
                    let children: Vec<SyntaxNode> = entry.named_children(&mut cursor).collect();
                    let labels: Vec<String> = children
                        .iter()
                        .filter(|child| child.kind() == "switch_pattern")
                        .map(|pattern| one_line(flow.text(*pattern)))
                        .collect();
                    let statements: Vec<SyntaxNode> = children
                        .iter()
                        .filter(|child| child.kind() == "statements")
                        .copied()
                        .collect();
                    // `fallthrough` is a word of the case's own.
                    let mut cursor = entry.walk();
                    let falls = entry
                        .children(&mut cursor)
                        .any(|child| child.kind() == "fallthrough");
                    Case {
                        label: if labels.is_empty() {
                            "default".into()
                        } else {
                            labels.join(", ")
                        },
                        statements,
                        falls,
                    }
                })
                .collect();
            let has_default = cases.iter().any(|case| case.label == "default");
            let value = field("expr").map_or(String::new(), |value| flow.text(value).to_string());
            let header = entries.first().map(|entry| entry.start_byte());
            flow.switch(
                statement,
                header,
                "Switch",
                &value,
                cases,
                has_default,
                exits,
            )
        }
        "do_statement" => {
            let body = child_of_kind(statement, "statements");
            let mut cursor = statement.walk();
            let handlers = statement
                .named_children(&mut cursor)
                .filter(|child| child.kind() == "catch_block")
                .map(|catch| {
                    let block = child_of_kind(catch, "statements");
                    let end = block.map_or(catch.end_byte(), |block| block.start_byte());
                    let caught = flow.head(catch.start_byte()..end, &["catch"]);
                    Handler {
                        label: if caught.is_empty() {
                            "catch".into()
                        } else {
                            format!("catch {caught}")
                        },
                        body: block,
                    }
                })
                .collect();
            flow.attempt(statement, body, handlers, None, None, exits)
        }
        "control_transfer_statement" => match transfer(flow, statement) {
            Some("return") => flow.end(statement, "Return", exits),
            Some("throw") => flow.end(statement, "Throw", exits),
            Some("continue") => flow.end(statement, "Continue", exits),
            Some("break") => flow.exit_loop(statement, exits),
            _ => exits,
        },
        "call_expression"
            if statement
                .named_child(0)
                .is_some_and(|callee| flow.text(callee) == "defer") =>
        {
            flow.step_named(statement, statement, |_| "Defer".into(), exits)
        }
        "function_declaration"
        | "class_declaration"
        | "import_declaration"
        | "typealias_declaration"
        | "protocol_declaration"
        | "comment"
        | "multiline_comment" => exits,
        _ => flow.step(statement, exits),
    }
}

/// `return`, `throw`, `break` or `continue`.
fn transfer<'s>(flow: &Flow<'s>, statement: SyntaxNode) -> Option<&'s str> {
    if statement.kind() != "control_transfer_statement" {
        return None;
    }
    statement.child(0).map(|keyword| flow.text(keyword))
}

fn binds<'t>(node: SyntaxNode<'t>, _: &str) -> Binds<'t> {
    let bound = |node: SyntaxNode<'t>| {
        let mut cursor = node.walk();
        node.children_by_field_name("bound_identifier", &mut cursor)
            .collect::<Vec<_>>()
    };
    match node.kind() {
        "property_declaration" => {
            let mut cursor = node.walk();
            Binds::Block(node.children_by_field_name("name", &mut cursor).collect())
        }
        // `guard let x` binds for what follows.
        "guard_statement" => Binds::Block(bound(node)),
        "if_statement" | "while_statement" => Binds::Inside(bound(node)),
        "for_statement" => Binds::Inside(node.child_by_field_name("item").into_iter().collect()),
        "lambda_literal" => {
            let mut parameters = Vec::new();
            let mut stack = vec![node];
            while let Some(node) = stack.pop() {
                if node.kind() == "lambda_parameter" {
                    parameters.extend(node.child_by_field_name("name"));
                    continue;
                }
                if node.kind() == "statements" {
                    continue;
                }
                let mut cursor = node.walk();
                stack.extend(node.named_children(&mut cursor));
            }
            Binds::Inside(parameters)
        }
        _ => Binds::Nothing,
    }
}

#[cfg(test)]
mod tests;
