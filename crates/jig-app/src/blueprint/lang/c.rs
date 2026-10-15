//! C and C++. Functions are definitions, in C++ also in classes, structs
//! and namespaces, and out-of-class ones like `Store::later`, listed under
//! `Store`. A declaration binds what it declares; a `case` runs on into
//! the next until a `break`; `goto` and `abort()` end a path; in C++, a
//! range `for` is a For Each and `try` has an output per `catch`.

use tree_sitter::Node as SyntaxNode;

use crate::blueprint::flow::data::{Binds, Names};
use crate::blueprint::flow::{Case, Exits, Flow, Handler, Lister, Parts, Scope, Syntax};
use crate::blueprint::{Function, one_line, text};

const NAMES: Names = Names {
    names: &["identifier"],
    scopes: &[
        "compound_statement",
        "for_statement",
        "for_range_loop",
        "if_statement",
        "while_statement",
    ],
    skip: &[
        "primitive_type",
        "type_identifier",
        "sized_type_specifier",
        "struct_specifier",
        "enum_specifier",
        "union_specifier",
        "class_specifier",
        "type_qualifier",
        "storage_class_specifier",
        "placeholder_type_specifier",
        "template_argument_list",
        "type_descriptor",
        "statement_identifier",
        "namespace_identifier",
        "qualified_identifier",
        "attribute_declaration",
    ],
    not_variables: &[("field_expression", "field")],
    defaults: &[(
        "optional_parameter_declaration",
        "declarator",
        "default_value",
    )],
    binds,
};

pub(in crate::blueprint) static C: Syntax = Syntax {
    separator: "::",
    functions,
    parts,
    statement,
    calls: &[("call_expression", "function")],
    members: &[("field_expression", "field")],
    news: &[],
    sets: &[
        ("assignment_expression", "left"),
        ("update_expression", "argument"),
    ],
    transparent: &["expression_statement", "parenthesized_expression"],
    panics: &["abort"],
    data: NAMES,
};

pub(in crate::blueprint) static CPP: Syntax = Syntax {
    separator: "::",
    functions,
    parts,
    statement,
    calls: &[("call_expression", "function")],
    members: &[
        ("field_expression", "field"),
        ("qualified_identifier", "name"),
        ("template_function", "name"),
        ("template_method", "name"),
    ],
    news: &[("new_expression", "type")],
    sets: &[
        ("assignment_expression", "left"),
        ("update_expression", "argument"),
    ],
    transparent: &[
        "expression_statement",
        "parenthesized_expression",
        "condition_clause",
        "co_await_expression",
    ],
    panics: &["abort"],
    data: NAMES,
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

fn statement(flow: &mut Flow, statement: SyntaxNode, exits: Exits) -> Exits {
    let field = |name: &str| statement.child_by_field_name(name);
    match statement.kind() {
        "compound_statement" => flow.block(statement, exits),
        "declaration" => {
            let names = declared(statement);
            if names.is_empty() {
                return exits;
            }
            let title = names
                .iter()
                .map(|name| one_line(flow.text(*name)))
                .collect::<Vec<_>>()
                .join(", ");
            let mut cursor = statement.walk();
            let value = statement
                .children_by_field_name("declarator", &mut cursor)
                .find_map(|declarator| declarator.child_by_field_name("value"));
            flow.bind(statement, format!("Let {title}"), value, exits)
        }
        "if_statement" => {
            let condition = flow.condition(field("condition"));
            let otherwise = field("alternative").and_then(|clause| match clause.kind() {
                "else_clause" => clause.named_child(0),
                _ => Some(clause),
            });
            flow.branch(
                statement,
                &condition,
                field("consequence"),
                otherwise,
                exits,
            )
        }
        "for_statement" | "for_range_loop" => {
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
        "switch_statement" => {
            let body = field("body");
            let cases: Vec<Case> = body
                .map(|body| {
                    let mut cursor = body.walk();
                    body.named_children(&mut cursor)
                        .filter(|case| case.kind() == "case_statement")
                        .map(|case| {
                            let value = case.child_by_field_name("value");
                            let mut cursor = case.walk();
                            Case {
                                label: value
                                    .map_or("default".into(), |value| one_line(flow.text(value))),
                                statements: case
                                    .named_children(&mut cursor)
                                    .filter(|child| Some(*child) != value)
                                    .collect(),
                                falls: true,
                            }
                        })
                        .collect()
                })
                .unwrap_or_default();
            let has_default = cases.iter().any(|case| case.label == "default");
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
        "try_statement" => {
            let mut cursor = statement.walk();
            let handlers = statement
                .named_children(&mut cursor)
                .filter(|clause| clause.kind() == "catch_clause")
                .map(|clause| Handler {
                    label: clause
                        .child_by_field_name("parameters")
                        .map_or("catch".into(), |caught| {
                            format!("catch {}", one_line(flow.text(caught)))
                        }),
                    body: clause.child_by_field_name("body"),
                })
                .collect();
            flow.attempt(statement, field("body"), handlers, None, None, exits)
        }
        "labeled_statement" => {
            let mut cursor = statement.walk();
            let inner = statement
                .named_children(&mut cursor)
                .filter(|child| child.kind() != "statement_identifier")
                .last();
            match inner {
                Some(inner) => flow.statement(inner, exits),
                None => exits,
            }
        }
        "return_statement" | "co_return_statement" => flow.end(statement, "Return", exits),
        "throw_statement" => flow.end(statement, "Throw", exits),
        "continue_statement" => flow.end(statement, "Continue", exits),
        "goto_statement" => flow.end(statement, "Goto", exits),
        "break_statement" => flow.exit_loop(statement, exits),
        "type_definition"
        | "struct_specifier"
        | "class_specifier"
        | "enum_specifier"
        | "using_declaration"
        | "alias_declaration"
        | "namespace_alias_definition"
        | "static_assert_declaration"
        | "comment"
        | ";" => exits,
        _ => flow.step(statement, exits),
    }
}

fn binds<'t>(node: SyntaxNode<'t>, _: &str) -> Binds<'t> {
    match node.kind() {
        "declaration" => Binds::Block(declared(node)),
        "for_range_loop" => {
            Binds::Inside(node.child_by_field_name("declarator").into_iter().collect())
        }
        "catch_clause" => {
            Binds::Inside(node.child_by_field_name("parameters").into_iter().collect())
        }
        "lambda_expression" => Binds::Inside(
            node.child_by_field_name("declarator")
                .and_then(|declarator| declarator.child_by_field_name("parameters"))
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
