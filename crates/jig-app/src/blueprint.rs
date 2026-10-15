//! The blueprint view's model: a Rust function read as blocks joined by
//! the order they run in, like an Unreal Blueprint's exec wires. Each
//! statement is a block; `if`, `match` and loops branch, and `return`,
//! `continue` and panics end a path. Read with tree-sitter, so it works on
//! code that doesn't build yet.

mod data;
pub mod debug;
pub mod layout;
pub mod view;

use std::ops::Range;

use tree_sitter::{Node as SyntaxNode, Parser, Tree};

/// Lines of code a block shows before an ellipsis.
const MAX_CODE_LINES: usize = 3;
/// Characters a match arm's pattern keeps as its pin's label.
const MAX_PIN_LABEL: usize = 24;

/// A function in the file, named as it's called: `Type::name` in an impl.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Function {
    /// As it's called: `Type::name`.
    pub name: String,
    /// Just its own name, for listing under its section.
    pub short: String,
    /// What it's in, for grouping the list: `Workspace` for an impl,
    /// `Display for Workspace` for a trait's, `trait Named`, `mod tests`,
    /// joined by ` › ` when they nest. `None` at the top of the file.
    pub section: Option<String>,
    /// The whole item, signature and body, as byte offsets.
    pub range: Range<usize>,
    /// Where its name starts, to ask a language server about it.
    pub name_at: usize,
}

/// A call to the function shown, from another function.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Caller {
    /// The function making the call.
    pub name: String,
    /// The file it's in, `None` for the one shown.
    pub path: Option<std::path::PathBuf>,
    /// Where the call is, as a byte offset in that file.
    pub at: usize,
    /// Where that is, for showing: `workspace.rs:412`.
    pub place: String,
}

/// What a block does, which sets its title's colour.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// The function itself, where the flow starts.
    Entry,
    Let,
    Call,
    /// Any other statement, or a block's value.
    Statement,
    Branch,
    Match,
    Loop,
    /// `return`, the function's tail value, `break`, `continue` or a panic.
    Exit,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Node {
    pub kind: Kind,
    pub title: String,
    /// The code it stands for, trimmed to a few lines.
    pub code: Vec<String>,
    /// Its exec outputs, by label; a plain next step is one unlabelled
    /// output, and an exit has none.
    pub outs: Vec<String>,
    /// The names it binds that later blocks read, each a data output.
    pub values: Vec<String>,
    /// The bindings it reads, by name, each a data input.
    pub inputs: Vec<String>,
    /// Where it is in the file, as byte offsets, for clicking through.
    pub range: Range<usize>,
    /// Where the name of the function it calls starts, to open that
    /// function's blueprint.
    pub callee: Option<usize>,
}

/// An exec wire: from output `out` of node `from` to node `to`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Edge {
    pub from: usize,
    pub out: usize,
    pub to: usize,
}

/// A data wire: value `value` of node `from` (a name its `let` or the
/// function binds) read through input `input` of node `to`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Data {
    pub from: usize,
    pub value: usize,
    pub to: usize,
    pub input: usize,
}

/// A function's flow. Every edge runs from an earlier node to a later one,
/// so the nodes' order is one they can be laid out in.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Graph {
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
    pub data: Vec<Data>,
}

/// Whether the blueprint view can read files in `language`.
pub fn reads(language: &str) -> bool {
    language == "rust"
}

fn parse(source: &str) -> Option<Tree> {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_rust::LANGUAGE.into())
        .ok()?;
    parser.parse(source, None)
}

/// The file's functions with bodies, in the order they're written,
/// including those in impls, traits' default methods and modules.
pub fn functions(source: &str) -> Vec<Function> {
    let Some(tree) = parse(source) else {
        return Vec::new();
    };
    let mut found = Vec::new();
    collect_functions(tree.root_node(), source, &Scope::default(), &mut found);
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
    fn within(&self, owner: Option<String>, section: String) -> Scope {
        Scope {
            owner: match (&self.owner, owner) {
                (Some(outer), Some(owner)) => Some(format!("{outer}::{owner}")),
                (outer, owner) => owner.or_else(|| outer.clone()),
            },
            section: Some(match &self.section {
                Some(outer) => format!("{outer} › {section}"),
                None => section,
            }),
        }
    }
}

fn collect_functions(node: SyntaxNode, source: &str, scope: &Scope, found: &mut Vec<Function>) {
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        let field = |name: &str| {
            child
                .child_by_field_name(name)
                .map(|node| text(node, source))
        };
        let inner = match child.kind() {
            "function_item" => {
                let Some(name) = child.child_by_field_name("name") else {
                    continue;
                };
                let short = text(name, source).to_string();
                found.push(Function {
                    name: match &scope.owner {
                        Some(owner) => format!("{owner}::{short}"),
                        None => short.clone(),
                    },
                    short,
                    section: scope.section.clone(),
                    range: child.byte_range(),
                    name_at: name.start_byte(),
                });
                continue;
            }
            "impl_item" => {
                let ty = field("type").map(base_type).unwrap_or_default();
                let section = match field("trait") {
                    Some(implemented) => format!("{} for {ty}", base_type(implemented)),
                    None => ty.clone(),
                };
                scope.within(Some(ty), section)
            }
            "trait_item" => {
                let name = field("name").unwrap_or_default().to_string();
                scope.within(Some(name.clone()), format!("trait {name}"))
            }
            "mod_item" => {
                let name = field("name").unwrap_or_default().to_string();
                scope.within(Some(name.clone()), format!("mod {name}"))
            }
            _ => continue,
        };
        if let Some(body) = child.child_by_field_name("body") {
            collect_functions(body, source, &inner, found);
        }
    }
}

/// `Workspace` from `Workspace<'a, T>`.
fn base_type(name: &str) -> String {
    name.split('<').next().unwrap_or(name).trim().to_string()
}

/// The function whose item contains `offset`, innermost first, else the
/// first one.
pub fn at(functions: &[Function], offset: usize) -> Option<usize> {
    containing(functions, offset).or((!functions.is_empty()).then_some(0))
}

/// The flow of the function at `function.range`, or `None` if it's no
/// longer there.
pub fn graph(source: &str, function: &Function) -> Option<Graph> {
    let tree = parse(source)?;
    let item = tree
        .root_node()
        .descendant_for_byte_range(function.range.start, function.range.end)?;
    let item = std::iter::successors(Some(item), |node| node.parent())
        .find(|node| node.kind() == "function_item")?;
    let mut builder = Builder {
        source,
        graph: Graph::default(),
        loops: Vec::new(),
    };
    let entry = builder.entry(item, function);
    if let Some(body) = item.child_by_field_name("body") {
        builder.block(body, vec![(entry, 0)], true);
    }
    data::add(&mut builder.graph, item, source);
    Some(builder.graph)
}

/// The outputs still waiting for a next step: (node, output).
type Exits = Vec<(usize, usize)>;

struct Builder<'a> {
    source: &'a str,
    graph: Graph,
    /// The `break`s of each loop being built, innermost last, which
    /// continue where the loop does.
    loops: Vec<Exits>,
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
        let mut code: Vec<String> = item
            .child_by_field_name("parameters")
            .map(|parameters| {
                let mut cursor = parameters.walk();
                parameters
                    .named_children(&mut cursor)
                    .filter(|parameter| !parameter.is_extra())
                    .map(|parameter| one_line(text(parameter, self.source)))
                    .collect()
            })
            .unwrap_or_default();
        if let Some(returns) = item.child_by_field_name("return_type") {
            code.push(format!("→ {}", one_line(text(returns, self.source))));
        }
        let end = item
            .child_by_field_name("body")
            .map_or(item.end_byte(), |body| body.start_byte());
        self.add(
            Node {
                kind: Kind::Entry,
                title: function.name.clone(),
                code: shorten(code),
                outs: vec![String::new()],
                values: Vec::new(),
                inputs: Vec::new(),
                range: item.start_byte()..end,
                callee: None,
            },
            &Vec::new(),
        )
    }

    /// The statements of `block` in order, continuing from `exits`. With
    /// `returns`, its value is the function's.
    fn block(&mut self, block: SyntaxNode, mut exits: Exits, returns: bool) -> Exits {
        let mut cursor = block.walk();
        let statements: Vec<_> = block
            .named_children(&mut cursor)
            .filter(|child| !child.is_extra())
            .collect();
        for (ix, statement) in statements.iter().enumerate() {
            let last = ix + 1 == statements.len();
            exits = match statement.kind() {
                // `if` and `match` are statements without a `;` too, and
                // the block's value when they come last.
                "expression_statement" => match statement.named_child(0) {
                    Some(expression) => {
                        let value = last && !ends_with_semicolon(*statement);
                        self.expression(expression, exits, returns && value)
                    }
                    None => exits,
                },
                "let_declaration" => {
                    let node = self.let_node(*statement);
                    vec![(self.add(node, &exits), 0)]
                }
                kind if is_declaration(kind) => exits,
                // A block's last expression, without a semicolon: its value.
                _ => self.expression(*statement, exits, returns && last),
            };
        }
        exits
    }

    fn expression(&mut self, expression: SyntaxNode, exits: Exits, returns: bool) -> Exits {
        match expression.kind() {
            "if_expression" => self.branch(expression, exits, returns),
            "match_expression" => self.matching(expression, exits, returns),
            "for_expression" | "while_expression" | "loop_expression" => {
                self.repeat(expression, exits)
            }
            "block" => self.block(expression, exits, returns),
            "unsafe_block" | "async_block" | "const_block" => {
                match named_child_of_kind(expression, "block") {
                    Some(block) => self.block(block, exits, returns),
                    None => self.statement(expression, exits, returns),
                }
            }
            "break_expression" => {
                let node = self.exit(expression, "Break");
                let at = self.add(
                    Node {
                        outs: vec![String::new()],
                        ..node
                    },
                    &exits,
                );
                match self.loops.last_mut() {
                    Some(breaks) => breaks.push((at, 0)),
                    // `break` out of a labelled block: on from there.
                    None => return vec![(at, 0)],
                }
                Vec::new()
            }
            "continue_expression" => {
                let node = self.exit(expression, "Continue");
                self.add(node, &exits);
                Vec::new()
            }
            "return_expression" => {
                let node = self.exit(expression, "Return");
                self.add(node, &exits);
                Vec::new()
            }
            "macro_invocation" if panics(expression, self.source) => {
                let node = self.exit(expression, "Panic");
                self.add(node, &exits);
                Vec::new()
            }
            _ => self.statement(expression, exits, returns),
        }
    }

    /// A step that just runs and goes on, or with `returns`, the
    /// function's value.
    fn statement(&mut self, expression: SyntaxNode, exits: Exits, returns: bool) -> Exits {
        if returns {
            let node = self.exit(expression, "Return");
            self.add(node, &exits);
            return Vec::new();
        }
        let (kind, title) = describe(expression, self.source);
        let callee = callee(expression);
        let node = Node {
            kind,
            title,
            code: code_lines(text(expression, self.source)),
            outs: vec![String::new()],
            values: Vec::new(),
            inputs: Vec::new(),
            range: statement_range(expression),
            callee,
        };
        vec![(self.add(node, &exits), 0)]
    }

    fn let_node(&self, statement: SyntaxNode) -> Node {
        let pattern = statement
            .child_by_field_name("pattern")
            .map_or("", |pattern| text(pattern, self.source));
        Node {
            kind: Kind::Let,
            title: format!("Let {}", one_line(pattern)),
            code: code_lines(text(statement, self.source)),
            outs: vec![String::new()],
            values: Vec::new(),
            inputs: Vec::new(),
            range: statement.byte_range(),
            callee: statement.child_by_field_name("value").and_then(callee),
        }
    }

    fn exit(&self, expression: SyntaxNode, title: &str) -> Node {
        Node {
            kind: Kind::Exit,
            title: title.to_string(),
            code: code_lines(text(expression, self.source)),
            outs: Vec::new(),
            values: Vec::new(),
            inputs: Vec::new(),
            range: statement_range(expression),
            callee: None,
        }
    }

    /// `if`, with an output for each way it can go; `else if` chains on
    /// from the first's False.
    fn branch(&mut self, expression: SyntaxNode, exits: Exits, returns: bool) -> Exits {
        let condition = expression
            .child_by_field_name("condition")
            .map_or("", |condition| text(condition, self.source));
        let header = expression
            .child_by_field_name("consequence")
            .map_or(expression.end_byte(), |block| block.start_byte());
        let at = self.add(
            Node {
                kind: Kind::Branch,
                title: "Branch".into(),
                code: code_lines(condition),
                outs: vec!["True".into(), "False".into()],
                values: Vec::new(),
                inputs: Vec::new(),
                range: expression.start_byte()..header,
                callee: None,
            },
            &exits,
        );
        let mut after = match expression.child_by_field_name("consequence") {
            Some(block) => self.block(block, vec![(at, 0)], returns),
            None => vec![(at, 0)],
        };
        let otherwise = expression
            .child_by_field_name("alternative")
            .and_then(|clause| clause.named_child(0));
        match otherwise {
            Some(otherwise) => after.extend(self.expression(otherwise, vec![(at, 1)], returns)),
            None => after.push((at, 1)),
        }
        after
    }

    /// `match`, with an output for each arm, labelled with its pattern.
    fn matching(&mut self, expression: SyntaxNode, exits: Exits, returns: bool) -> Exits {
        let value = expression
            .child_by_field_name("value")
            .map_or("", |value| text(value, self.source));
        let arms: Vec<_> = expression
            .child_by_field_name("body")
            .map(|body| {
                let mut cursor = body.walk();
                body.named_children(&mut cursor)
                    .filter(|arm| arm.kind() == "match_arm")
                    .collect()
            })
            .unwrap_or_default();
        let header = expression
            .child_by_field_name("body")
            .map_or(expression.end_byte(), |body| body.start_byte());
        let at = self.add(
            Node {
                kind: Kind::Match,
                title: "Match".into(),
                code: code_lines(value),
                outs: arms
                    .iter()
                    .map(|arm| {
                        let pattern = arm
                            .child_by_field_name("pattern")
                            .map_or("_", |pattern| text(pattern, self.source));
                        clip(&one_line(pattern), MAX_PIN_LABEL)
                    })
                    .collect(),
                values: Vec::new(),
                inputs: Vec::new(),
                range: expression.start_byte()..header,
                callee: None,
            },
            &exits,
        );
        let mut after = Vec::new();
        for (ix, arm) in arms.iter().enumerate() {
            match arm.child_by_field_name("value") {
                Some(value) => after.extend(self.expression(value, vec![(at, ix)], returns)),
                None => after.push((at, ix)),
            }
        }
        after
    }

    /// A loop: its body runs from Body, and what follows from Completed
    /// and each `break`. The body's end loops back, which isn't drawn.
    fn repeat(&mut self, expression: SyntaxNode, exits: Exits) -> Exits {
        let (title, head, completes) = match expression.kind() {
            "for_expression" => {
                let pattern = expression
                    .child_by_field_name("pattern")
                    .map_or("", |node| text(node, self.source));
                let value = expression
                    .child_by_field_name("value")
                    .map_or("", |node| text(node, self.source));
                ("For Each", format!("{pattern} in {value}"), true)
            }
            "while_expression" => {
                let condition = expression
                    .child_by_field_name("condition")
                    .map_or("", |node| text(node, self.source));
                ("While", condition.to_string(), true)
            }
            _ => ("Loop", String::new(), false),
        };
        let body = expression.child_by_field_name("body");
        let mut outs = vec!["Body".to_string()];
        if completes {
            outs.push("Completed".into());
        }
        let at = self.add(
            Node {
                kind: Kind::Loop,
                title: title.into(),
                code: code_lines(&head),
                outs,
                values: Vec::new(),
                inputs: Vec::new(),
                range: expression.start_byte()
                    ..body.map_or(expression.end_byte(), |body| body.start_byte()),
                callee: None,
            },
            &exits,
        );
        self.loops.push(Vec::new());
        if let Some(body) = body {
            self.block(body, vec![(at, 0)], false);
        }
        let mut after = self.loops.pop().unwrap_or_default();
        if completes {
            after.insert(0, (at, 1));
        }
        after
    }
}

/// Items, `use`s and attributes inside a body, which don't run in order.
fn is_declaration(kind: &str) -> bool {
    kind.ends_with("_item")
        || kind == "use_declaration"
        || kind == "attribute_item"
        || kind == "inner_attribute_item"
        || kind == "empty_statement"
        || kind == "label"
}

fn ends_with_semicolon(statement: SyntaxNode) -> bool {
    statement
        .child(statement.child_count().saturating_sub(1) as u32)
        .is_some_and(|last| last.kind() == ";")
}

fn named_child_of_kind<'t>(node: SyntaxNode<'t>, kind: &str) -> Option<SyntaxNode<'t>> {
    let mut cursor = node.walk();
    node.named_children(&mut cursor)
        .find(|child| child.kind() == kind)
}

/// The statement around an expression, so its `;` is selected too.
fn statement_range(expression: SyntaxNode) -> Range<usize> {
    match expression.parent() {
        Some(parent) if parent.kind() == "expression_statement" => parent.byte_range(),
        _ => expression.byte_range(),
    }
}

fn panics(invocation: SyntaxNode, source: &str) -> bool {
    let name = invocation
        .child_by_field_name("macro")
        .map_or("", |name| text(name, source));
    matches!(
        name.rsplit("::").next().unwrap_or(name),
        "panic" | "unreachable" | "todo" | "unimplemented"
    )
}

/// A statement's kind and title: what it calls, sets or evaluates.
fn describe(expression: SyntaxNode, source: &str) -> (Kind, String) {
    match expression.kind() {
        // `x?` and `x.await` are about `x`.
        "try_expression" | "await_expression" => match expression.named_child(0) {
            Some(inner) => describe(inner, source),
            None => (Kind::Statement, "Statement".into()),
        },
        "call_expression" => {
            let callee = expression
                .child_by_field_name("function")
                .map_or("", |callee| callee_name(callee, source));
            (Kind::Call, format!("Call {callee}"))
        }
        "macro_invocation" => {
            let name = expression
                .child_by_field_name("macro")
                .map_or("", |name| text(name, source));
            (Kind::Call, format!("{name}!"))
        }
        "assignment_expression" | "compound_assignment_expr" => {
            let left = expression
                .child_by_field_name("left")
                .map_or("", |left| text(left, source));
            (
                Kind::Statement,
                format!("Set {}", clip(&one_line(left), 32)),
            )
        }
        _ => (Kind::Statement, "Statement".into()),
    }
}

/// Where the name of the function `expression` calls starts, if it's a
/// call: `push` in `self.items.push(x)?`.
fn callee(expression: SyntaxNode) -> Option<usize> {
    match expression.kind() {
        "try_expression" | "await_expression" => callee(expression.named_child(0)?),
        "call_expression" => callee_name_node(expression.child_by_field_name("function")?)
            .map(|name| name.start_byte()),
        _ => None,
    }
}

/// The name in a callee: `push` in `self.items.push`, `new` in `Vec::new`.
fn callee_name_node(callee: SyntaxNode) -> Option<SyntaxNode> {
    match callee.kind() {
        "field_expression" => callee.child_by_field_name("field"),
        "generic_function" => callee_name_node(callee.child_by_field_name("function")?),
        "scoped_identifier" => callee.child_by_field_name("name"),
        _ => Some(callee),
    }
}

/// The calls to `function` in `source`, each with the function making it,
/// for when no language server says. Calls are matched by name, so a
/// method of another type with the same name counts too.
pub fn callers_in(source: &str, function: &Function) -> Vec<(String, usize)> {
    let Some(tree) = parse(source) else {
        return Vec::new();
    };
    let short = function.name.rsplit("::").next().unwrap_or(&function.name);
    let functions = functions(source);
    let mut calls = Vec::new();
    let mut stack = vec![tree.root_node()];
    while let Some(node) = stack.pop() {
        if node.kind() == "call_expression"
            && let Some(name) = node
                .child_by_field_name("function")
                .and_then(callee_name_node)
            && text(name, source) == short
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

/// The innermost function whose item contains `offset`.
fn containing(functions: &[Function], offset: usize) -> Option<usize> {
    functions
        .iter()
        .enumerate()
        .filter(|(_, function)| function.range.contains(&offset))
        .min_by_key(|(_, function)| function.range.len())
        .map(|(ix, _)| ix)
}

/// The block whose code holds `offset`, the smallest if they nest.
pub fn node_at(graph: &Graph, offset: usize) -> Option<usize> {
    graph
        .nodes
        .iter()
        .enumerate()
        .filter(|(_, node)| node.range.contains(&offset))
        .min_by_key(|(_, node)| node.range.len())
        .map(|(ix, _)| ix)
}

/// What a call calls: `push` for `self.items.push`, `Vec::new` as it is.
fn callee_name<'s>(callee: SyntaxNode, source: &'s str) -> &'s str {
    match callee.kind() {
        "field_expression" => callee
            .child_by_field_name("field")
            .map_or("", |field| text(field, source)),
        "generic_function" => callee
            .child_by_field_name("function")
            .map_or("", |function| callee_name(function, source)),
        _ => text(callee, source),
    }
}

fn text<'s>(node: SyntaxNode, source: &'s str) -> &'s str {
    source.get(node.byte_range()).unwrap_or("")
}

fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn clip(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let kept: String = text.chars().take(max - 1).collect();
    format!("{}…", kept.trim_end())
}

/// The code's first few non-blank lines, without their indentation.
fn code_lines(code: &str) -> Vec<String> {
    shorten(
        code.lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(str::to_string)
            .collect(),
    )
}

fn shorten(mut lines: Vec<String>) -> Vec<String> {
    if lines.len() > MAX_CODE_LINES {
        lines.truncate(MAX_CODE_LINES);
        if let Some(last) = lines.last_mut() {
            last.push_str(" …");
        }
    }
    lines
}

#[cfg(test)]
mod tests;
