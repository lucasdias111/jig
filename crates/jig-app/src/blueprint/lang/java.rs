//! Java. Functions are methods and constructors (`Store.save`), in
//! classes, interfaces, enums and records, nested or not. A `case …:`
//! group runs on into the next until a `break`, a `case … ->` rule
//! doesn't; `try` has an output per `catch`, all on through `finally`.

use tree_sitter::Node as SyntaxNode;

use crate::blueprint::flow::data::{Binds, Names};
use crate::blueprint::flow::{Case, Exits, Flow, Handler, Lister, Parts, Scope, Syntax};
use crate::blueprint::{Function, one_line, text};

pub(in crate::blueprint) static SYNTAX: Syntax = Syntax {
    separator: ".",
    functions,
    parts,
    statement,
    calls: &[
        ("method_invocation", "name"),
        ("explicit_constructor_invocation", "constructor"),
    ],
    members: &[("generic_type", ""), ("scoped_type_identifier", "")],
    news: &[("object_creation_expression", "type")],
    sets: &[("assignment_expression", "left"), ("update_expression", "")],
    transparent: &["expression_statement", "parenthesized_expression"],
    panics: &[],
    data: Names {
        names: &["identifier"],
        scopes: &[
            "block",
            "for_statement",
            "enhanced_for_statement",
            "switch_block_statement_group",
            "switch_rule",
        ],
        skip: &[
            "type_identifier",
            "integral_type",
            "floating_point_type",
            "boolean_type",
            "void_type",
            "generic_type",
            "array_type",
            "scoped_type_identifier",
            "catch_type",
            "dimensions",
            "modifiers",
            "annotation",
            "marker_annotation",
            "type_arguments",
        ],
        not_variables: &[("method_invocation", "name"), ("field_access", "field")],
        defaults: &[],
        binds,
    },
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

fn statement(flow: &mut Flow, statement: SyntaxNode, exits: Exits) -> Exits {
    let field = |name: &str| statement.child_by_field_name(name);
    match statement.kind() {
        "block" | "constructor_body" => flow.block(statement, exits),
        "local_variable_declaration" => {
            let declarators = declarators(statement);
            let names: Vec<&str> = declarators
                .iter()
                .filter_map(|d| d.child_by_field_name("name"))
                .map(|name| flow.text(name))
                .collect();
            let value = declarators
                .first()
                .and_then(|d| d.child_by_field_name("value"));
            flow.bind(statement, format!("Let {}", names.join(", ")), value, exits)
        }
        "if_statement" => {
            let condition = flow.condition(field("condition"));
            flow.branch(
                statement,
                &condition,
                field("consequence"),
                field("alternative"),
                exits,
            )
        }
        "for_statement" | "enhanced_for_statement" => {
            let body = field("body");
            let end = body.map_or(statement.end_byte(), |body| body.start_byte());
            let head = flow.head(statement.start_byte()..end, &["for"]);
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
        "switch_expression" | "switch_statement" => {
            let body = field("body");
            let mut cases: Vec<Case> = Vec::new();
            if let Some(body) = body {
                let mut cursor = body.walk();
                for group in body.named_children(&mut cursor) {
                    let rule = group.kind() == "switch_rule";
                    if !rule && group.kind() != "switch_block_statement_group" {
                        continue;
                    }
                    let mut labels = Vec::new();
                    let mut statements = Vec::new();
                    let mut cursor = group.walk();
                    for child in group.named_children(&mut cursor) {
                        if child.kind() == "switch_label" {
                            let label = one_line(flow.text(child));
                            labels.push(label.strip_prefix("case ").unwrap_or(&label).to_string());
                        } else {
                            statements.push(child);
                        }
                    }
                    cases.push(Case {
                        label: labels.join(", "),
                        statements,
                        falls: !rule,
                    });
                }
            }
            let has_default = cases
                .iter()
                .any(|case| case.label.split(", ").any(|label| label == "default"));
            let value = flow.condition(field("condition"));
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
        "try_statement" | "try_with_resources_statement" => {
            let mut handlers = Vec::new();
            let mut finally = None;
            let mut cursor = statement.walk();
            for clause in statement.named_children(&mut cursor) {
                match clause.kind() {
                    "catch_clause" => {
                        let caught = clause
                            .named_child(0)
                            .and_then(|parameter| parameter.named_child(0))
                            .filter(|ty| ty.kind() == "catch_type");
                        handlers.push(Handler {
                            label: match caught {
                                Some(ty) => format!("catch {}", one_line(flow.text(ty))),
                                None => "catch".into(),
                            },
                            body: clause.child_by_field_name("body"),
                        });
                    }
                    "finally_clause" => finally = clause.named_child(0),
                    _ => {}
                }
            }
            flow.attempt(statement, field("body"), handlers, None, finally, exits)
        }
        "synchronized_statement" => match field("body") {
            Some(body) => flow.statement(body, exits),
            None => exits,
        },
        "labeled_statement" => {
            match statement.named_child(statement.named_child_count().saturating_sub(1) as u32) {
                Some(inner) if inner.kind() != "identifier" => flow.statement(inner, exits),
                _ => exits,
            }
        }
        "return_statement" => flow.end(statement, "Return", exits),
        "throw_statement" => flow.end(statement, "Throw", exits),
        "yield_statement" => flow.end(statement, "Yield", exits),
        "continue_statement" => flow.end(statement, "Continue", exits),
        "break_statement" => flow.exit_loop(statement, exits),
        "local_class_declaration"
        | "class_declaration"
        | "record_declaration"
        | "interface_declaration"
        | "enum_declaration"
        | ";"
        | "line_comment"
        | "block_comment" => exits,
        _ => flow.step(statement, exits),
    }
}

fn binds<'t>(node: SyntaxNode<'t>, _: &str) -> Binds<'t> {
    let field = |name: &str| node.child_by_field_name(name);
    match node.kind() {
        "local_variable_declaration" => Binds::Block(
            declarators(node)
                .iter()
                .filter_map(|d| d.child_by_field_name("name"))
                .collect(),
        ),
        "enhanced_for_statement" => Binds::Inside(field("name").into_iter().collect()),
        "catch_clause" => Binds::Inside(
            node.named_child(0)
                .and_then(|parameter| parameter.child_by_field_name("name"))
                .into_iter()
                .collect(),
        ),
        "lambda_expression" => Binds::Inside(match field("parameters") {
            Some(parameters) if parameters.kind() == "identifier" => vec![parameters],
            Some(parameters) => {
                let mut cursor = parameters.walk();
                parameters.named_children(&mut cursor).collect()
            }
            None => Vec::new(),
        }),
        _ => Binds::Nothing,
    }
}

#[cfg(test)]
mod tests;
