//! Zig. Functions are `fn`s, at the top or in a `const Store = struct
//! { … }` (`Store.method`). `const` and `var` bind, and so do the
//! `|payloads|` of loops, `if` and `catch`, for their own body; an
//! assignment is a Set; `switch` has an output per prong; `defer` and
//! `errdefer` are steps; `@panic` ends a path.

use tree_sitter::Node as SyntaxNode;

use crate::blueprint::flow::data::{Binds, Names};
use crate::blueprint::flow::{Case, Exits, Flow, Lister, Parts, Scope, Syntax};
use crate::blueprint::{Function, one_line, text};

pub(in crate::blueprint) static SYNTAX: Syntax = Syntax {
    separator: ".",
    functions,
    parts,
    statement,
    calls: &[("call_expression", "function"), ("builtin_function", "")],
    members: &[("field_expression", "member")],
    news: &[],
    sets: &[],
    transparent: &[
        "expression_statement",
        "try_expression",
        "parenthesized_expression",
    ],
    panics: &["@panic"],
    data: Names {
        names: &["identifier"],
        scopes: &["block"],
        skip: &[
            "builtin_type",
            "pointer_type",
            "error_union_type",
            "optional_type",
            "slice_type",
            "array_type",
            "builtin_identifier",
        ],
        not_variables: &[
            ("parameter", "type"),
            ("variable_declaration", "type"),
            ("function_declaration", "type"),
            ("field_expression", "member"),
        ],
        defaults: &[],
        binds,
    },
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
        returns_body: false,
    })
}

/// `const` or `var`, if the declaration starts with one; an assignment
/// reads as a declaration without.
fn keyword<'s>(flow: &Flow<'s>, declaration: SyntaxNode) -> Option<&'s str> {
    let word = flow.text(declaration.child(0)?);
    matches!(word, "const" | "var").then_some(word)
}

fn statement(flow: &mut Flow, statement: SyntaxNode, exits: Exits) -> Exits {
    let field = |name: &str| statement.child_by_field_name(name);
    match statement.kind() {
        "block" => flow.block(statement, exits),
        "block_expression" | "labeled_statement" => {
            let mut cursor = statement.walk();
            let inner: Vec<SyntaxNode> = statement
                .named_children(&mut cursor)
                .filter(|child| child.kind() != "block_label")
                .collect();
            flow.statements(inner, exits)
        }
        "variable_declaration" => match keyword(flow, statement) {
            Some(word) => {
                let mut cursor = statement.walk();
                let named: Vec<SyntaxNode> = statement.named_children(&mut cursor).collect();
                let name = named
                    .iter()
                    .find(|n| n.kind() == "identifier")
                    .map_or("", |name| flow.text(*name));
                let title = format!("{}{} {name}", word[..1].to_uppercase(), &word[1..]);
                let value = named
                    .last()
                    .copied()
                    .filter(|last| last.kind() != "identifier");
                flow.bind(statement, title, value, exits)
            }
            None => {
                let target = statement
                    .named_child(0)
                    .map_or(String::new(), |target| one_line(flow.text(target)));
                flow.step_named(statement, statement, |_| format!("Set {target}"), exits)
            }
        },
        "expression_statement" => {
            let inner = flow.unwrap(statement);
            match inner.kind() {
                "return_expression" | "break_expression" | "continue_expression" => {
                    ends(flow, statement, inner.kind(), exits)
                }
                _ => flow.step(statement, exits),
            }
        }
        kind @ ("return_expression" | "break_expression" | "continue_expression") => {
            ends(flow, statement, kind, exits)
        }
        "if_statement" => {
            let condition = flow.condition(field("condition"));
            let otherwise = child_of_kind(statement, "else_clause")
                .and_then(|clause| clause.child_by_field_name("alternative"));
            flow.branch(statement, &condition, field("body"), otherwise, exits)
        }
        "for_statement" | "while_statement" => {
            let body = field("body");
            let mut cursor = statement.walk();
            let head: Vec<String> = statement
                .named_children(&mut cursor)
                .filter(|child| Some(*child) != body)
                .map(|child| one_line(flow.text(child)))
                .collect();
            let title = if statement.kind() == "for_statement" {
                "For Each"
            } else {
                "While"
            };
            flow.loop_on(statement, title, &head.join(" "), body, true, exits)
        }
        "switch_expression" => {
            let mut cursor = statement.walk();
            let named: Vec<SyntaxNode> = statement.named_children(&mut cursor).collect();
            let prongs: Vec<SyntaxNode> = named
                .iter()
                .filter(|child| child.kind() == "switch_case")
                .copied()
                .collect();
            let cases: Vec<Case> = prongs
                .iter()
                .map(|prong| {
                    let mut cursor = prong.walk();
                    let parts: Vec<SyntaxNode> = prong.named_children(&mut cursor).collect();
                    let (result, labels) = parts
                        .split_last()
                        .map_or((None, &[][..]), |(last, rest)| (Some(*last), rest));
                    let mut cursor = prong.walk();
                    let is_else = prong
                        .children(&mut cursor)
                        .any(|child| child.kind() == "else");
                    Case {
                        label: if is_else {
                            "else".into()
                        } else {
                            labels
                                .iter()
                                .map(|label| one_line(flow.text(*label)))
                                .collect::<Vec<_>>()
                                .join(", ")
                        },
                        statements: result.into_iter().collect(),
                        falls: false,
                    }
                })
                .collect();
            let has_default = cases.iter().any(|case| case.label == "else");
            let value = named
                .first()
                .filter(|value| value.kind() != "switch_case")
                .map_or(String::new(), |value| flow.text(*value).to_string());
            let header = prongs.first().map(|prong| prong.start_byte());
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
        "defer_statement" | "errdefer_statement" => {
            let inner = statement.named_child(0).unwrap_or(statement);
            let word = if statement.kind() == "defer_statement" {
                "Defer"
            } else {
                "Errdefer"
            };
            flow.step_named(
                statement,
                inner,
                |said| match said.strip_prefix("Call ") {
                    Some(called) => format!("{word} {called}"),
                    None => word.into(),
                },
                exits,
            )
        }
        "function_declaration" | "test_declaration" | "comment" => exits,
        _ => flow.step(statement, exits),
    }
}

fn ends(flow: &mut Flow, statement: SyntaxNode, kind: &str, exits: Exits) -> Exits {
    match kind {
        "return_expression" => flow.end(statement, "Return", exits),
        "continue_expression" => flow.end(statement, "Continue", exits),
        _ => flow.exit_loop(statement, exits),
    }
}

/// The names in `node`'s `|payload|`s, but not its body's.
fn payloads(node: SyntaxNode) -> Vec<SyntaxNode> {
    let mut cursor = node.walk();
    node.named_children(&mut cursor)
        .filter(|child| child.kind() == "payload")
        .flat_map(|payload| {
            let mut cursor = payload.walk();
            payload
                .named_children(&mut cursor)
                .filter(|name| name.kind() == "identifier")
                .collect::<Vec<_>>()
        })
        .collect()
}

fn binds<'t>(node: SyntaxNode<'t>, source: &str) -> Binds<'t> {
    match node.kind() {
        "variable_declaration" => {
            let declares = node
                .child(0)
                .is_some_and(|word| matches!(text(word, source), "const" | "var"));
            if !declares {
                return Binds::Nothing;
            }
            let mut cursor = node.walk();
            let name = node
                .named_children(&mut cursor)
                .find(|child| child.kind() == "identifier");
            Binds::Block(name.into_iter().collect())
        }
        "for_statement" | "while_statement" | "if_statement" | "catch_expression"
        | "else_clause" => Binds::Inside(payloads(node)),
        _ => Binds::Nothing,
    }
}

#[cfg(test)]
mod tests;
