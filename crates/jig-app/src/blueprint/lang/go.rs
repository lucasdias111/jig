//! Go. Functions are `func`s and methods (`Store.Save`, listed under their
//! receiver's type). `:=`, `var` and `const` bind; `if` may start with a
//! statement of its own; `for` is a For, a For Each over a `range`, a
//! While with just a condition, or a Loop with nothing, which only a
//! `break` leaves; a `switch` case runs on into the next only with
//! `fallthrough`; `select` has an output per case; `defer` and `go` are
//! steps; `panic(…)` ends a path.

use tree_sitter::Node as SyntaxNode;

use crate::blueprint::flow::data::{Binds, Names};
use crate::blueprint::flow::{Case, Exits, Flow, Lister, Parts, Scope, Syntax, label_of};
use crate::blueprint::{Function, one_line, text};

pub(in crate::blueprint) static SYNTAX: Syntax = Syntax {
    separator: ".",
    functions,
    parts,
    statement,
    calls: &[("call_expression", "function")],
    members: &[("selector_expression", "field")],
    news: &[],
    sets: &[
        ("assignment_statement", "left"),
        ("inc_statement", ""),
        ("dec_statement", ""),
    ],
    transparent: &["expression_statement", "parenthesized_expression"],
    panics: &["panic"],
    data: Names {
        names: &["identifier"],
        scopes: &[
            "block",
            "for_statement",
            "if_statement",
            "expression_switch_statement",
            "type_switch_statement",
            "expression_case",
            "type_case",
            "default_case",
            "communication_case",
        ],
        skip: &[
            "type_identifier",
            "pointer_type",
            "qualified_type",
            "slice_type",
            "map_type",
            "array_type",
            "channel_type",
            "function_type",
            "generic_type",
            "interface_type",
            "struct_type",
            "label_name",
            "package_identifier",
        ],
        not_variables: &[("selector_expression", "field")],
        defaults: &[],
        binds,
    },
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

fn statement(flow: &mut Flow, statement: SyntaxNode, exits: Exits) -> Exits {
    let field = |name: &str| statement.child_by_field_name(name);
    match statement.kind() {
        "block" => flow.block(statement, exits),
        "short_var_declaration" => {
            let left = field("left").map_or("", |left| flow.text(left));
            let title = format!("Let {}", one_line(left));
            let value = field("right").and_then(|right| right.named_child(0));
            flow.bind(statement, title, value, exits)
        }
        "var_declaration" | "const_declaration" => {
            let keyword = if statement.kind() == "var_declaration" {
                "Var"
            } else {
                "Const"
            };
            let specs = specs(statement);
            let names: Vec<&str> = specs
                .iter()
                .flat_map(|spec| names_of(*spec))
                .map(|name| flow.text(name))
                .collect();
            let value = specs
                .first()
                .and_then(|spec| spec.child_by_field_name("value"))
                .and_then(|value| value.named_child(0));
            flow.bind(
                statement,
                format!("{keyword} {}", names.join(", ")),
                value,
                exits,
            )
        }
        "if_statement" => {
            let condition = flow.condition(field("condition"));
            let condition = match field("initializer") {
                Some(start) => format!("{}; {condition}", flow.text(start)),
                None => condition,
            };
            flow.branch(
                statement,
                &condition,
                field("consequence"),
                field("alternative"),
                exits,
            )
        }
        "for_statement" => {
            let body = field("body");
            let head = statement.named_child(0).filter(|head| Some(*head) != body);
            let (title, completes) = match head.map(|head| head.kind()) {
                Some("range_clause") => ("For Each", true),
                Some("for_clause") => ("For", true),
                Some(_) => ("While", true),
                None => ("Loop", false),
            };
            let head = head.map_or("", |head| flow.text(head)).to_string();
            flow.loop_on(statement, title, &head, body, completes, exits)
        }
        "expression_switch_statement" | "type_switch_statement" | "select_statement" => {
            let mut cursor = statement.walk();
            let clauses: Vec<SyntaxNode> = statement
                .named_children(&mut cursor)
                .filter(|child| {
                    matches!(
                        child.kind(),
                        "expression_case" | "type_case" | "default_case" | "communication_case"
                    )
                })
                .collect();
            let header = clauses.first().map(|case| case.start_byte());
            let cases: Vec<Case> = clauses.iter().map(|clause| case(flow, *clause)).collect();
            let has_default = clauses.iter().any(|case| case.kind() == "default_case");
            let (title, value) = match statement.kind() {
                "select_statement" => ("Select", String::new()),
                "type_switch_statement" => (
                    "Switch",
                    field("value").map_or(String::new(), |value| flow.text(value).to_string()),
                ),
                _ => ("Switch", flow.condition(field("value"))),
            };
            flow.switch(statement, header, title, &value, cases, has_default, exits)
        }
        "defer_statement" | "go_statement" => {
            let word = if statement.kind() == "defer_statement" {
                "Defer"
            } else {
                "Go"
            };
            let inner = statement.named_child(0).unwrap_or(statement);
            flow.step_named(
                statement,
                inner,
                |said| format!("{word} {}", said.trim_start_matches("Call ")),
                exits,
            )
        }
        "labeled_statement" => {
            let mut cursor = statement.walk();
            let inner = statement
                .named_children(&mut cursor)
                .filter(|child| child.kind() != "label_name")
                .last();
            match inner {
                Some(inner) => flow.statement(inner, exits),
                None => exits,
            }
        }
        "return_statement" => flow.end(statement, "Return", exits),
        "continue_statement" => flow.end(statement, "Continue", exits),
        "goto_statement" => flow.end(statement, "Goto", exits),
        "break_statement" => flow.exit_loop(statement, exits),
        "fallthrough_statement" | "empty_statement" | "type_declaration" | "comment" => exits,
        _ => flow.step(statement, exits),
    }
}

/// A case of a switch or select: its label, statements, and whether it
/// ends in `fallthrough`.
fn case<'t>(flow: &Flow, clause: SyntaxNode<'t>) -> Case<'t> {
    let mut labels = Vec::new();
    let mut statements = Vec::new();
    let mut cursor = clause.walk();
    for (ix, child) in clause.children(&mut cursor).enumerate() {
        if !child.is_named() || child.is_extra() {
            continue;
        }
        match clause.field_name_for_child(ix as u32) {
            Some("value" | "type" | "communication") => labels.push(child),
            _ => statements.push(child),
        }
    }
    let falls = statements
        .last()
        .is_some_and(|last| last.kind() == "fallthrough_statement");
    Case {
        label: if clause.kind() == "default_case" {
            "default".into()
        } else {
            label_of(flow, &labels)
        },
        statements,
        falls,
    }
}

fn specs(declaration: SyntaxNode) -> Vec<SyntaxNode> {
    let mut cursor = declaration.walk();
    declaration
        .named_children(&mut cursor)
        .flat_map(|child| match child.kind() {
            // `var ( … )`
            "var_spec_list" | "const_spec_list" => {
                let mut cursor = child.walk();
                child.named_children(&mut cursor).collect::<Vec<_>>()
            }
            _ => vec![child],
        })
        .filter(|spec| matches!(spec.kind(), "var_spec" | "const_spec"))
        .collect()
}

fn names_of(spec: SyntaxNode) -> Vec<SyntaxNode> {
    let mut cursor = spec.walk();
    spec.children_by_field_name("name", &mut cursor).collect()
}

fn binds<'t>(node: SyntaxNode<'t>, source: &str) -> Binds<'t> {
    let field = |name: &str| node.child_by_field_name(name);
    match node.kind() {
        "short_var_declaration" => Binds::Block(field("left").into_iter().collect()),
        "var_declaration" | "const_declaration" => {
            Binds::Block(specs(node).into_iter().flat_map(names_of).collect())
        }
        // `v := <-ch` in a select.
        "receive_statement" if text(node, source).contains(":=") => {
            Binds::Block(field("left").into_iter().collect())
        }
        "for_statement" => {
            let range = node
                .named_child(0)
                .filter(|clause| clause.kind() == "range_clause")
                .filter(|clause| text(*clause, source).contains(":="));
            Binds::Inside(
                range
                    .and_then(|range| range.child_by_field_name("left"))
                    .into_iter()
                    .collect(),
            )
        }
        "type_switch_statement" => Binds::Inside(field("alias").into_iter().collect()),
        "func_literal" => Binds::Inside(
            field("parameters")
                .map(|list| {
                    let mut cursor = list.walk();
                    list.named_children(&mut cursor).collect()
                })
                .unwrap_or_default(),
        ),
        _ => Binds::Nothing,
    }
}

#[cfg(test)]
mod tests;
