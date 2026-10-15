//! JavaScript and TypeScript (and TSX). Functions are declarations, `const`
//! and `let` bindings of arrow functions and function expressions, and
//! class methods (`Class.method`), in classes and namespaces. `switch` runs
//! on into the next case until a `break`; `try` has Body and Catch, both on
//! through `finally`; an arrow function's expression is its Return.

use tree_sitter::Node as SyntaxNode;

use crate::blueprint::flow::data::{Binds, Names};
use crate::blueprint::flow::{Case, Exits, Flow, Handler, Lister, Parts, Scope, Syntax};
use crate::blueprint::{Function, text};

pub(in crate::blueprint) static SYNTAX: Syntax = Syntax {
    separator: ".",
    functions,
    parts,
    statement,
    calls: &[("call_expression", "function")],
    members: &[
        ("member_expression", "property"),
        ("non_null_expression", ""),
    ],
    news: &[("new_expression", "constructor")],
    sets: &[
        ("assignment_expression", "left"),
        ("augmented_assignment_expression", "left"),
        ("update_expression", "argument"),
    ],
    transparent: &[
        "expression_statement",
        "await_expression",
        "parenthesized_expression",
        "non_null_expression",
    ],
    panics: &[],
    data: Names {
        names: &[
            "identifier",
            "shorthand_property_identifier",
            "shorthand_property_identifier_pattern",
        ],
        scopes: &["statement_block", "for_statement", "switch_body"],
        skip: &[
            "type_annotation",
            "type_arguments",
            "type_parameters",
            "type_alias_declaration",
            "interface_declaration",
            "statement_identifier",
            "this",
        ],
        not_variables: &[],
        defaults: &[
            ("assignment_pattern", "left", "right"),
            ("object_assignment_pattern", "left", "right"),
            ("required_parameter", "pattern", "value"),
            ("optional_parameter", "pattern", "value"),
        ],
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
        returns_body: body.is_some_and(|body| body.kind() != "statement_block"),
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

fn statement(flow: &mut Flow, statement: SyntaxNode, exits: Exits) -> Exits {
    let field = |name: &str| statement.child_by_field_name(name);
    match statement.kind() {
        "statement_block" => flow.block(statement, exits),
        "lexical_declaration" | "variable_declaration" => {
            let keyword = match statement.child(0).map(|keyword| flow.text(keyword)) {
                Some("const") => "Const",
                Some("let") => "Let",
                _ => "Var",
            };
            let declarators = declarators(statement);
            let names: Vec<String> = declarators
                .iter()
                .filter_map(|d| d.child_by_field_name("name"))
                .map(|name| crate::blueprint::one_line(flow.text(name)))
                .collect();
            let value = declarators
                .first()
                .and_then(|d| d.child_by_field_name("value"));
            flow.bind(
                statement,
                format!("{keyword} {}", names.join(", ")),
                value,
                exits,
            )
        }
        "if_statement" => {
            let condition = flow.condition(field("condition"));
            let otherwise = field("alternative").and_then(|clause| clause.named_child(0));
            flow.branch(
                statement,
                &condition,
                field("consequence"),
                otherwise,
                exits,
            )
        }
        "switch_statement" => {
            let body = field("body");
            let cases: Vec<Case> = body
                .map(|body| {
                    let mut cursor = body.walk();
                    body.named_children(&mut cursor)
                        .filter(|case| matches!(case.kind(), "switch_case" | "switch_default"))
                        .map(|case| {
                            let mut cursor = case.walk();
                            Case {
                                label: case
                                    .child_by_field_name("value")
                                    .map_or("default".into(), |value| flow.text(value).into()),
                                statements: case
                                    .children_by_field_name("body", &mut cursor)
                                    .collect(),
                                falls: true,
                            }
                        })
                        .collect()
                })
                .unwrap_or_default();
            let has_default = cases.iter().any(|case| case.label == "default");
            let value = flow.condition(field("value"));
            let header = body.map(|body| body.start_byte());
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
        "for_statement" | "for_in_statement" => {
            let body = field("body");
            let end = body.map_or(statement.end_byte(), |body| body.start_byte());
            let head = flow.head(statement.start_byte()..end, &["for", "await"]);
            let title = if statement.kind() == "for_statement" {
                "For"
            } else {
                "For Each"
            };
            flow.loop_on(statement, title, &head, body, true, exits)
        }
        "while_statement" | "do_statement" => {
            let condition = flow.condition(field("condition"));
            let title = if statement.kind() == "while_statement" {
                "While"
            } else {
                "Do While"
            };
            flow.loop_on(statement, title, &condition, field("body"), true, exits)
        }
        "try_statement" => {
            let handlers = field("handler")
                .map(|handler| Handler {
                    label: "Catch".into(),
                    body: handler.child_by_field_name("body"),
                })
                .into_iter()
                .collect();
            let finally =
                field("finalizer").and_then(|finally| finally.child_by_field_name("body"));
            flow.attempt(statement, field("body"), handlers, None, finally, exits)
        }
        "labeled_statement" => match field("body") {
            Some(body) => flow.statement(body, exits),
            None => exits,
        },
        "return_statement" => flow.end(statement, "Return", exits),
        "throw_statement" => flow.end(statement, "Throw", exits),
        "continue_statement" => flow.end(statement, "Continue", exits),
        "break_statement" => flow.exit_loop(statement, exits),
        "function_declaration"
        | "generator_function_declaration"
        | "class_declaration"
        | "abstract_class_declaration"
        | "interface_declaration"
        | "type_alias_declaration"
        | "enum_declaration"
        | "import_statement"
        | "export_statement"
        | "ambient_declaration"
        | "empty_statement"
        | "comment" => exits,
        _ => flow.step(statement, exits),
    }
}

fn binds<'t>(node: SyntaxNode<'t>, _: &str) -> Binds<'t> {
    match node.kind() {
        "lexical_declaration" | "variable_declaration" => Binds::Block(
            declarators(node)
                .iter()
                .filter_map(|d| d.child_by_field_name("name"))
                .collect(),
        ),
        // `for (const x of xs)`; without a keyword, it sets one already
        // bound.
        "for_in_statement" if node.child_by_field_name("kind").is_some() => {
            Binds::Inside(node.child_by_field_name("left").into_iter().collect())
        }
        kind if is_function(kind) => Binds::Inside(parameters(node)),
        "catch_clause" => {
            Binds::Inside(node.child_by_field_name("parameter").into_iter().collect())
        }
        _ => Binds::Nothing,
    }
}

#[cfg(test)]
mod tests;
