//! JavaScript and TypeScript (and TSX) read as blueprints. Functions are
//! declarations, `const` and `let` bindings of arrow functions and function
//! expressions, and class methods (`Class.method`), in classes and
//! namespaces. In a body, `if` branches, `switch` has an output per case
//! (falling through to the next until a `break`), loops have Body and
//! Completed, `try` has Body and Catch, and `return`, `throw`, `break` and
//! `continue` end a path.

mod data;

use tree_sitter::Node as SyntaxNode;

use super::{
    Edge, Function, Graph, Kind, MAX_PIN_LABEL, Node, clip, code_lines, containing, one_line,
    shorten, text,
};

/// The functions in the file, in the order they're written.
pub(super) fn functions(root: SyntaxNode, source: &str) -> Vec<Function> {
    let mut found = Vec::new();
    collect(root, source, &Scope::default(), &mut found);
    found
}

/// Where functions being collected are: the prefix their names take, and
/// the section they're listed under.
#[derive(Clone, Default)]
struct Scope {
    owner: Option<String>,
    section: Option<String>,
}

impl Scope {
    fn within(&self, owner: &str, section: String) -> Scope {
        Scope {
            owner: Some(match &self.owner {
                Some(outer) => format!("{outer}.{owner}"),
                None => owner.to_string(),
            }),
            section: Some(match &self.section {
                Some(outer) => format!("{outer} › {section}"),
                None => section,
            }),
        }
    }

    fn function(&self, short: &str, item: SyntaxNode, name: SyntaxNode) -> Function {
        Function {
            name: match &self.owner {
                Some(owner) => format!("{owner}.{short}"),
                None => short.to_string(),
            },
            short: short.to_string(),
            section: self.section.clone(),
            range: item.byte_range(),
            name_at: name.start_byte(),
        }
    }
}

fn collect(node: SyntaxNode, source: &str, scope: &Scope, found: &mut Vec<Function>) {
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        let name = child.child_by_field_name("name");
        match child.kind() {
            "export_statement" => collect(child, source, scope, found),
            "function_declaration" | "generator_function_declaration" => {
                if let Some(name) = name {
                    found.push(scope.function(text(name, source), child, name));
                }
            }
            "lexical_declaration" | "variable_declaration" => {
                let mut cursor = child.walk();
                let declarators: Vec<_> = child
                    .named_children(&mut cursor)
                    .filter(|d| d.kind() == "variable_declarator")
                    .collect();
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
                        found.push(scope.function(text(name, source), item, name));
                    }
                }
            }
            "class_declaration" | "abstract_class_declaration" => {
                let (Some(name), Some(body)) = (name, child.child_by_field_name("body")) else {
                    continue;
                };
                let class = text(name, source);
                let inner = scope.within(class, format!("class {class}"));
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
                        found.push(inner.function(text(member_name, source), member, member_name));
                    }
                }
            }
            "module" | "internal_module" => {
                let (Some(name), Some(body)) = (name, child.child_by_field_name("body")) else {
                    continue;
                };
                let namespace = text(name, source);
                let inner = scope.within(namespace, format!("namespace {namespace}"));
                collect(body, source, &inner, found);
            }
            // `declare namespace …` and `namespace …;` as an expression.
            "ambient_declaration" | "expression_statement" => collect(child, source, scope, found),
            _ => {}
        }
    }
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

/// The function itself in the item at `function.range`: the item, or the
/// function a `const` or a class field holds.
fn function_node<'t>(root: SyntaxNode<'t>, function: &Function) -> Option<SyntaxNode<'t>> {
    let range = &function.range;
    let found = root.descendant_for_byte_range(range.start, range.end)?;
    let item = std::iter::successors(Some(found), |node| node.parent())
        .find(|node| node.byte_range() == *range)?;
    if is_function(item.kind()) {
        return Some(item);
    }
    let mut stack = vec![item];
    while let Some(node) = stack.pop() {
        let value = node.child_by_field_name("value");
        let name = node.child_by_field_name("name");
        if let Some(value) = value
            && is_function(value.kind())
            && name.is_none_or(|name| name.start_byte() == function.name_at)
        {
            return Some(value);
        }
        let mut cursor = node.walk();
        stack.extend(node.named_children(&mut cursor));
    }
    None
}

/// The flow of the function at `function.range`, or `None` if it's no
/// longer there.
pub(super) fn graph(root: SyntaxNode, source: &str, function: &Function) -> Option<Graph> {
    let item = function_node(root, function)?;
    let mut builder = Builder {
        source,
        graph: Graph::default(),
        breaks: Vec::new(),
    };
    let entry = builder.entry(item, function);
    match item.child_by_field_name("body") {
        Some(body) if body.kind() == "statement_block" => {
            builder.block(body, vec![(entry, 0)]);
        }
        // An arrow function's expression: its value.
        Some(body) => {
            let node = builder.exit(body, "Return");
            builder.add(node, &vec![(entry, 0)]);
        }
        None => {}
    }
    data::add(&mut builder.graph, item, source);
    Some(builder.graph)
}

/// The outputs still waiting for a next step: (node, output).
type Exits = Vec<(usize, usize)>;

struct Builder<'a> {
    source: &'a str,
    graph: Graph,
    /// The `break`s of each loop or `switch` being built, innermost last,
    /// which continue after it.
    breaks: Vec<Exits>,
}

impl Builder<'_> {
    fn add(&mut self, node: Node, from: &Exits) -> usize {
        let to = self.graph.nodes.len();
        self.graph.nodes.push(node);
        self.graph
            .edges
            .extend(from.iter().map(|&(from, out)| Edge { from, out, to }));
        to
    }

    fn entry(&mut self, item: SyntaxNode, function: &Function) -> usize {
        let parameters = match item.child_by_field_name("parameters") {
            Some(parameters) => {
                let mut cursor = parameters.walk();
                parameters
                    .named_children(&mut cursor)
                    .filter(|parameter| !parameter.is_extra())
                    .map(|parameter| one_line(text(parameter, self.source)))
                    .collect()
            }
            // `x => …`
            None => item
                .child_by_field_name("parameter")
                .map(|parameter| vec![text(parameter, self.source).to_string()])
                .unwrap_or_default(),
        };
        let mut code: Vec<String> = parameters;
        if let Some(returns) = item.child_by_field_name("return_type") {
            let returns = text(returns, self.source).trim_start_matches(':');
            code.push(format!("→ {}", one_line(returns)));
        }
        let start = function.range.start;
        let end = item
            .child_by_field_name("body")
            .map_or(function.range.end, |body| body.start_byte());
        self.add(
            Node {
                kind: Kind::Entry,
                title: function.name.clone(),
                code: shorten(code),
                outs: vec![String::new()],
                values: Vec::new(),
                inputs: Vec::new(),
                range: start..end,
                callee: None,
            },
            &Vec::new(),
        )
    }

    /// The statements of `block` in order, continuing from `exits`.
    fn block(&mut self, block: SyntaxNode, mut exits: Exits) -> Exits {
        let mut cursor = block.walk();
        let statements: Vec<_> = block
            .named_children(&mut cursor)
            .filter(|child| !child.is_extra())
            .collect();
        for statement in statements {
            exits = self.statement(statement, exits);
        }
        exits
    }

    fn statement(&mut self, statement: SyntaxNode, exits: Exits) -> Exits {
        match statement.kind() {
            "statement_block" => self.block(statement, exits),
            "lexical_declaration" | "variable_declaration" => {
                let node = self.declaration(statement);
                vec![(self.add(node, &exits), 0)]
            }
            "if_statement" => self.branch(statement, exits),
            "switch_statement" => self.switch(statement, exits),
            "for_statement" | "for_in_statement" | "while_statement" | "do_statement" => {
                self.repeat(statement, exits)
            }
            "try_statement" => self.attempt(statement, exits),
            "labeled_statement" => match statement.child_by_field_name("body") {
                Some(body) => self.statement(body, exits),
                None => exits,
            },
            "return_statement" => self.end(statement, "Return", exits),
            "throw_statement" => self.end(statement, "Throw", exits),
            "continue_statement" => self.end(statement, "Continue", exits),
            "break_statement" => {
                let node = Node {
                    outs: vec![String::new()],
                    ..self.exit(statement, "Break")
                };
                let at = self.add(node, &exits);
                match self.breaks.last_mut() {
                    Some(breaks) => breaks.push((at, 0)),
                    // Out of a labelled block: on from there.
                    None => return vec![(at, 0)],
                }
                Vec::new()
            }
            kind if is_declaration(kind) => exits,
            _ => {
                let expression = match statement.kind() {
                    "expression_statement" => statement.named_child(0).unwrap_or(statement),
                    _ => statement,
                };
                let (kind, title) = describe(expression, self.source);
                let node = Node {
                    kind,
                    title,
                    code: code_lines(text(statement, self.source)),
                    outs: vec![String::new()],
                    values: Vec::new(),
                    inputs: Vec::new(),
                    range: statement.byte_range(),
                    callee: callee(expression),
                };
                vec![(self.add(node, &exits), 0)]
            }
        }
    }

    /// A block that ends the path.
    fn end(&mut self, statement: SyntaxNode, title: &str, exits: Exits) -> Exits {
        let node = self.exit(statement, title);
        self.add(node, &exits);
        Vec::new()
    }

    fn exit(&self, statement: SyntaxNode, title: &str) -> Node {
        Node {
            kind: Kind::Exit,
            title: title.to_string(),
            code: code_lines(text(statement, self.source)),
            outs: Vec::new(),
            values: Vec::new(),
            inputs: Vec::new(),
            range: statement.byte_range(),
            callee: None,
        }
    }

    /// `const`, `let` or `var`, with what it binds.
    fn declaration(&self, statement: SyntaxNode) -> Node {
        let keyword = match statement.child(0).map(|keyword| text(keyword, self.source)) {
            Some("const") => "Const",
            Some("let") => "Let",
            _ => "Var",
        };
        let mut cursor = statement.walk();
        let declarators: Vec<_> = statement
            .named_children(&mut cursor)
            .filter(|d| d.kind() == "variable_declarator")
            .collect();
        let names: Vec<String> = declarators
            .iter()
            .filter_map(|d| d.child_by_field_name("name"))
            .map(|name| one_line(text(name, self.source)))
            .collect();
        Node {
            kind: Kind::Let,
            title: format!("{keyword} {}", names.join(", ")),
            code: code_lines(text(statement, self.source)),
            outs: vec![String::new()],
            values: Vec::new(),
            inputs: Vec::new(),
            range: statement.byte_range(),
            callee: declarators
                .first()
                .and_then(|d| d.child_by_field_name("value"))
                .and_then(callee),
        }
    }

    /// `if`, with True and False; `else if` chains on from the False.
    fn branch(&mut self, statement: SyntaxNode, exits: Exits) -> Exits {
        let consequence = statement.child_by_field_name("consequence");
        let at = self.add(
            Node {
                kind: Kind::Branch,
                title: "Branch".into(),
                code: code_lines(&self.inner(statement.child_by_field_name("condition"))),
                outs: vec!["True".into(), "False".into()],
                values: Vec::new(),
                inputs: Vec::new(),
                range: statement.start_byte()
                    ..consequence.map_or(statement.end_byte(), |c| c.start_byte()),
                callee: None,
            },
            &exits,
        );
        let mut after = match consequence {
            Some(consequence) => self.statement(consequence, vec![(at, 0)]),
            None => vec![(at, 0)],
        };
        let otherwise = statement
            .child_by_field_name("alternative")
            .and_then(|clause| clause.named_child(0));
        match otherwise {
            Some(otherwise) => after.extend(self.statement(otherwise, vec![(at, 1)])),
            None => after.push((at, 1)),
        }
        after
    }

    /// `switch`, with an output per case. A case runs on into the next
    /// until a `break`; without a `default`, a Default output goes on.
    fn switch(&mut self, statement: SyntaxNode, exits: Exits) -> Exits {
        let body = statement.child_by_field_name("body");
        let cases: Vec<SyntaxNode> = body
            .map(|body| {
                let mut cursor = body.walk();
                body.named_children(&mut cursor)
                    .filter(|case| matches!(case.kind(), "switch_case" | "switch_default"))
                    .collect()
            })
            .unwrap_or_default();
        let mut outs: Vec<String> = cases
            .iter()
            .map(|case| match case.child_by_field_name("value") {
                Some(value) => clip(&one_line(text(value, self.source)), MAX_PIN_LABEL),
                None => "default".into(),
            })
            .collect();
        let has_default = cases.iter().any(|case| case.kind() == "switch_default");
        if !has_default {
            outs.push("Default".into());
        }
        let at = self.add(
            Node {
                kind: Kind::Match,
                title: "Switch".into(),
                code: code_lines(&self.inner(statement.child_by_field_name("value"))),
                outs,
                values: Vec::new(),
                inputs: Vec::new(),
                range: statement.start_byte()
                    ..body.map_or(statement.end_byte(), |b| b.start_byte()),
                callee: None,
            },
            &exits,
        );
        self.breaks.push(Vec::new());
        let mut falling: Exits = Vec::new();
        for (ix, case) in cases.iter().enumerate() {
            let mut from = vec![(at, ix)];
            from.append(&mut falling);
            let mut cursor = case.walk();
            let statements: Vec<_> = case
                .children_by_field_name("body", &mut cursor)
                .filter(|child| !child.is_extra())
                .collect();
            for statement in statements {
                from = self.statement(statement, from);
            }
            falling = from;
        }
        let mut after = self.breaks.pop().unwrap_or_default();
        after.extend(falling);
        if !has_default {
            after.push((at, cases.len()));
        }
        after
    }

    /// A loop: its body runs from Body, and what follows from Completed
    /// and each `break`. The body's end loops back, which isn't drawn.
    fn repeat(&mut self, statement: SyntaxNode, exits: Exits) -> Exits {
        let body = statement.child_by_field_name("body");
        let (title, head) = match statement.kind() {
            "for_statement" => ("For", self.loop_head(statement, body)),
            "for_in_statement" => ("For Each", self.loop_head(statement, body)),
            "do_statement" => (
                "Do While",
                self.inner(statement.child_by_field_name("condition")),
            ),
            _ => (
                "While",
                self.inner(statement.child_by_field_name("condition")),
            ),
        };
        let at = self.add(
            Node {
                kind: Kind::Loop,
                title: title.into(),
                code: code_lines(&head),
                outs: vec!["Body".into(), "Completed".into()],
                values: Vec::new(),
                inputs: Vec::new(),
                range: statement.start_byte()
                    ..body.map_or(statement.end_byte(), |b| b.start_byte()),
                callee: None,
            },
            &exits,
        );
        self.breaks.push(Vec::new());
        if let Some(body) = body {
            self.statement(body, vec![(at, 0)]);
        }
        let mut after = self.breaks.pop().unwrap_or_default();
        after.insert(0, (at, 1));
        after
    }

    /// `try`, its body from Body and its `catch` from Catch, both on
    /// through `finally`.
    fn attempt(&mut self, statement: SyntaxNode, exits: Exits) -> Exits {
        let body = statement.child_by_field_name("body");
        let handler = statement.child_by_field_name("handler");
        let mut outs = vec!["Body".to_string()];
        if handler.is_some() {
            outs.push("Catch".into());
        }
        let at = self.add(
            Node {
                kind: Kind::Branch,
                title: "Try".into(),
                code: Vec::new(),
                outs,
                values: Vec::new(),
                inputs: Vec::new(),
                range: statement.start_byte()
                    ..body.map_or(statement.end_byte(), |b| b.start_byte()),
                callee: None,
            },
            &exits,
        );
        let mut after = match body {
            Some(body) => self.block(body, vec![(at, 0)]),
            None => vec![(at, 0)],
        };
        if let Some(handler) = handler {
            match handler.child_by_field_name("body") {
                Some(body) => after.extend(self.block(body, vec![(at, 1)])),
                None => after.push((at, 1)),
            }
        }
        match statement
            .child_by_field_name("finalizer")
            .and_then(|finally| finally.child_by_field_name("body"))
        {
            Some(finally) => self.block(finally, after),
            None => after,
        }
    }

    /// What's inside a condition's parentheses.
    fn inner(&self, node: Option<SyntaxNode>) -> String {
        let Some(node) = node else {
            return String::new();
        };
        let inside = match node.kind() {
            "parenthesized_expression" => node.named_child(0).unwrap_or(node),
            _ => node,
        };
        text(inside, self.source).to_string()
    }

    /// A `for` loop's head, between its parentheses.
    fn loop_head(&self, statement: SyntaxNode, body: Option<SyntaxNode>) -> String {
        let end = body.map_or(statement.end_byte(), |body| body.start_byte());
        let head = self.source.get(statement.start_byte()..end).unwrap_or("");
        let head = head.trim_start_matches("for").trim_start();
        let head = head.strip_prefix("await").unwrap_or(head).trim();
        let head = head.strip_prefix('(').unwrap_or(head);
        let head = head.trim_end().strip_suffix(')').unwrap_or(head);
        one_line(head)
    }
}

/// Declarations inside a body, which don't run in order.
fn is_declaration(kind: &str) -> bool {
    matches!(
        kind,
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
            | "comment"
    )
}

/// A statement's kind and title: what it calls, sets or evaluates.
fn describe(expression: SyntaxNode, source: &str) -> (Kind, String) {
    match expression.kind() {
        // `await x` and `x!` are about `x`.
        "await_expression" | "non_null_expression" | "parenthesized_expression" => {
            match expression.named_child(0) {
                Some(inner) => describe(inner, source),
                None => (Kind::Statement, "Statement".into()),
            }
        }
        "call_expression" => {
            let name = expression
                .child_by_field_name("function")
                .and_then(callee_name_node)
                .map_or("", |name| text(name, source));
            (Kind::Call, format!("Call {name}"))
        }
        "new_expression" => {
            let class = expression
                .child_by_field_name("constructor")
                .map_or("", |class| text(class, source));
            (Kind::Call, format!("New {}", clip(&one_line(class), 32)))
        }
        "assignment_expression" | "augmented_assignment_expression" => {
            let left = expression
                .child_by_field_name("left")
                .map_or("", |left| text(left, source));
            (
                Kind::Statement,
                format!("Set {}", clip(&one_line(left), 32)),
            )
        }
        "update_expression" => {
            let target = expression
                .child_by_field_name("argument")
                .map_or("", |target| text(target, source));
            (
                Kind::Statement,
                format!("Set {}", clip(&one_line(target), 32)),
            )
        }
        _ => (Kind::Statement, "Statement".into()),
    }
}

/// Where the name of the function `expression` calls starts, if it's a
/// call: `push` in `await this.items.push(x)`.
fn callee(expression: SyntaxNode) -> Option<usize> {
    match expression.kind() {
        "await_expression" | "non_null_expression" | "parenthesized_expression" => {
            callee(expression.named_child(0)?)
        }
        "call_expression" => callee_name_node(expression.child_by_field_name("function")?)
            .map(|name| name.start_byte()),
        "new_expression" => callee_name_node(expression.child_by_field_name("constructor")?)
            .map(|name| name.start_byte()),
        _ => None,
    }
}

/// The name in a callee: `push` in `this.items.push`, `go` in `go`.
fn callee_name_node(callee: SyntaxNode) -> Option<SyntaxNode> {
    match callee.kind() {
        "member_expression" => callee.child_by_field_name("property"),
        "non_null_expression" => callee_name_node(callee.named_child(0)?),
        "identifier" => Some(callee),
        _ => None,
    }
}

/// The calls to `function` in the file, each with the function making it,
/// matched by name.
pub(super) fn callers_in(
    root: SyntaxNode,
    source: &str,
    function: &Function,
) -> Vec<(String, usize)> {
    let functions = functions(root, source);
    let mut calls = Vec::new();
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        if node.kind() == "call_expression"
            && let Some(name) = node
                .child_by_field_name("function")
                .and_then(callee_name_node)
            && text(name, source) == function.short
            && let Some(caller) = containing(&functions, name.start_byte())
        {
            calls.push((functions[caller].name.clone(), name.start_byte()));
        }
        let mut cursor = node.walk();
        stack.extend(node.named_children(&mut cursor));
    }
    calls.sort_by_key(|(_, at)| *at);
    calls
}

#[cfg(test)]
mod tests;
