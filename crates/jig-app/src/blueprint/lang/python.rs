//! Python. Functions are `def`s, decorated or not, and methods
//! (`Class.method`) in classes. An assignment to a name binds it (`Set x`,
//! which data wires start from); `elif` chains on from the False; a loop's
//! `else` runs when it completes, not when it breaks; `try` has an output
//! per `except`, its `else` after the body, all on through `finally`;
//! `with` runs its body; `match` has an output per case.

use tree_sitter::Node as SyntaxNode;

use crate::blueprint::flow::data::{Binds, Names};
use crate::blueprint::flow::{Case, Exits, Flow, Handler, Lister, Parts, Scope, Syntax};
use crate::blueprint::{Function, Kind, one_line, text};

pub(in crate::blueprint) static SYNTAX: Syntax = Syntax {
    separator: ".",
    functions,
    parts,
    statement,
    calls: &[("call", "function")],
    members: &[("attribute", "attribute")],
    news: &[],
    sets: &[("assignment", "left"), ("augmented_assignment", "left")],
    transparent: &["expression_statement", "await", "parenthesized_expression"],
    panics: &[],
    data: Names {
        names: &["identifier"],
        // A name is seen in the whole function, not just its block.
        scopes: &[],
        skip: &["type", "decorator"],
        not_variables: &[("attribute", "attribute"), ("keyword_argument", "name")],
        defaults: &[
            ("default_parameter", "name", "value"),
            ("typed_default_parameter", "name", "value"),
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
        returns_body: false,
    })
}

/// Whether an assignment binds names rather than setting a field.
fn binds_names(assignment: SyntaxNode) -> bool {
    assignment.kind() == "assignment"
        && assignment.child_by_field_name("left").is_some_and(|left| {
            matches!(
                left.kind(),
                "identifier" | "pattern_list" | "tuple_pattern" | "list_pattern"
            )
        })
}

fn statement(flow: &mut Flow, statement: SyntaxNode, exits: Exits) -> Exits {
    let field = |name: &str| statement.child_by_field_name(name);
    match statement.kind() {
        "block" => flow.block(statement, exits),
        "expression_statement" => {
            let inner = flow.unwrap(statement);
            if binds_names(inner) {
                let left = inner
                    .child_by_field_name("left")
                    .map_or("", |left| flow.text(left));
                let title = format!("Set {}", one_line(left));
                let value = inner.child_by_field_name("right");
                flow.bind(statement, title, value, exits)
            } else {
                flow.step(statement, exits)
            }
        }
        "if_statement" => {
            let mut cursor = statement.walk();
            let rest: Vec<SyntaxNode> = statement
                .children_by_field_name("alternative", &mut cursor)
                .collect();
            let condition = flow.condition(field("condition"));
            chain(
                flow,
                statement,
                &condition,
                field("consequence"),
                &rest,
                exits,
            )
        }
        "for_statement" | "while_statement" => {
            let body = field("body");
            let (title, head) = if statement.kind() == "for_statement" {
                let left = field("left").map_or("", |left| flow.text(left));
                let right = field("right").map_or("", |right| flow.text(right));
                ("For Each", format!("{left} in {right}"))
            } else {
                ("While", flow.condition(field("condition")))
            };
            let (completed, breaks) = flow.repeat(statement, title, &head, body, true, exits);
            let mut after = match else_body(field("alternative")) {
                Some(otherwise) => flow.statement(otherwise, completed),
                None => completed,
            };
            after.extend(breaks);
            after
        }
        "try_statement" => {
            let mut handlers = Vec::new();
            let mut otherwise = None;
            let mut finally = None;
            let mut cursor = statement.walk();
            for clause in statement.named_children(&mut cursor) {
                match clause.kind() {
                    "except_clause" | "except_group_clause" => {
                        let caught = clause
                            .child_by_field_name("value")
                            .or_else(|| clause.named_child(0).filter(|c| c.kind() != "block"));
                        handlers.push(Handler {
                            label: match caught {
                                Some(caught) => format!("except {}", one_line(flow.text(caught))),
                                None => "except".into(),
                            },
                            body: last_block(clause),
                        });
                    }
                    "else_clause" => otherwise = clause.child_by_field_name("body"),
                    "finally_clause" => finally = last_block(clause),
                    _ => {}
                }
            }
            flow.attempt(
                statement,
                field("body"),
                handlers,
                otherwise,
                finally,
                exits,
            )
        }
        "with_statement" => {
            let body = field("body");
            let header = statement
                .named_child(0)
                .map_or(String::new(), |clause| flow.text(clause).to_string());
            let at = flow.fork(
                statement,
                body.map(|body| body.start_byte()),
                Kind::Statement,
                "With",
                &header,
                vec![String::new()],
                &exits,
            );
            match body {
                Some(body) => flow.statement(body, vec![(at, 0)]),
                None => vec![(at, 0)],
            }
        }
        "match_statement" => {
            let body = field("body");
            let cases: Vec<Case> = body
                .map(|body| {
                    let mut cursor = body.walk();
                    body.named_children(&mut cursor)
                        .filter(|case| case.kind() == "case_clause")
                        .map(|case| {
                            let mut cursor = case.walk();
                            let patterns: Vec<String> = case
                                .named_children(&mut cursor)
                                .filter(|child| child.kind() == "case_pattern")
                                .map(|pattern| one_line(flow.text(pattern)))
                                .collect();
                            Case {
                                label: patterns.join(", "),
                                statements: case
                                    .child_by_field_name("consequence")
                                    .into_iter()
                                    .collect(),
                                falls: false,
                            }
                        })
                        .collect()
                })
                .unwrap_or_default();
            let has_default = cases.iter().any(|case| case.label == "_");
            let subject = flow.condition(field("subject"));
            let header = body.map(|body| body.start_byte());
            flow.switch(
                statement,
                header,
                "Match",
                &subject,
                cases,
                has_default,
                exits,
            )
        }
        "return_statement" => flow.end(statement, "Return", exits),
        "raise_statement" => flow.end(statement, "Raise", exits),
        "continue_statement" => flow.end(statement, "Continue", exits),
        "break_statement" => flow.exit_loop(statement, exits),
        "pass_statement"
        | "function_definition"
        | "class_definition"
        | "decorated_definition"
        | "import_statement"
        | "import_from_statement"
        | "future_import_statement"
        | "global_statement"
        | "nonlocal_statement"
        | "comment" => exits,
        _ => flow.step(statement, exits),
    }
}

/// `if`, then each `elif` from the one before's False, then `else`.
fn chain(
    flow: &mut Flow,
    at: SyntaxNode,
    condition: &str,
    consequence: Option<SyntaxNode>,
    rest: &[SyntaxNode],
    exits: Exits,
) -> Exits {
    flow.branch_with(at, condition, consequence, exits, |flow, from| {
        match rest.split_first() {
            Some((clause, more)) if clause.kind() == "elif_clause" => {
                let condition = flow.condition(clause.child_by_field_name("condition"));
                let consequence = clause.child_by_field_name("consequence");
                chain(flow, *clause, &condition, consequence, more, from)
            }
            Some((clause, _)) => match else_body(Some(*clause)) {
                Some(body) => flow.statement(body, from),
                None => from,
            },
            None => from,
        }
    })
}

fn else_body(clause: Option<SyntaxNode>) -> Option<SyntaxNode> {
    clause?.child_by_field_name("body")
}

/// An `except` or `finally` clause's block, which has no field.
fn last_block(clause: SyntaxNode) -> Option<SyntaxNode> {
    let mut cursor = clause.walk();
    clause
        .named_children(&mut cursor)
        .filter(|child| child.kind() == "block")
        .last()
}

/// Every `as` target under `node`: `f` in `with open(p) as f`.
fn as_targets(node: SyntaxNode) -> Vec<SyntaxNode> {
    let mut found = Vec::new();
    let mut stack = vec![node];
    while let Some(node) = stack.pop() {
        if node.kind() == "as_pattern_target" {
            found.push(node);
            continue;
        }
        if node.kind() == "block" {
            continue;
        }
        let mut cursor = node.walk();
        stack.extend(node.named_children(&mut cursor));
    }
    found
}

fn binds<'t>(node: SyntaxNode<'t>, _: &str) -> Binds<'t> {
    match node.kind() {
        "assignment" if binds_names(node) => {
            Binds::Block(node.child_by_field_name("left").into_iter().collect())
        }
        "for_statement" => Binds::Inside(node.child_by_field_name("left").into_iter().collect()),
        "function_definition" | "lambda" => {
            let parameters = node.child_by_field_name("parameters");
            Binds::Inside(
                parameters
                    .map(|parameters| {
                        let mut cursor = parameters.walk();
                        parameters.named_children(&mut cursor).collect()
                    })
                    .unwrap_or_default(),
            )
        }
        "with_statement" | "except_clause" | "except_group_clause" => {
            Binds::Inside(as_targets(node))
        }
        // A comprehension's own names are seen only inside it.
        "list_comprehension"
        | "set_comprehension"
        | "dictionary_comprehension"
        | "generator_expression" => {
            let mut cursor = node.walk();
            let lefts = node
                .named_children(&mut cursor)
                .filter(|child| child.kind() == "for_in_clause")
                .filter_map(|clause| clause.child_by_field_name("left"))
                .collect();
            Binds::Inside(lefts)
        }
        _ => Binds::Nothing,
    }
}

#[cfg(test)]
mod tests;
