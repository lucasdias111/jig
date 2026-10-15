//! The flow of a function in any language but Rust, built the same way
//! for each: the language says, in a `Syntax`, which of its nodes are
//! calls, assignments and the like, how it lists its functions, and how
//! its statements map onto the shapes here (a step, a branch, a switch, a
//! loop, a try, an end). Everything else (blocks, wires, `break`s, the
//! entry block, data wires, callers by name) is shared.

pub(super) mod data;

use std::ops::Range;

use tree_sitter::Node as SyntaxNode;

use super::{
    Edge, Function, Graph, Kind, MAX_PIN_LABEL, Node, clip, code_lines, containing, one_line,
    shorten, text,
};

/// The outputs still waiting for a next step: (node, output).
pub(super) type Exits = Vec<(usize, usize)>;

/// What a language's syntax looks like, as far as blueprints go.
pub(super) struct Syntax {
    /// Between an owner's name and its function's: `Store.save`.
    pub separator: &'static str,
    /// The file's functions, in the order they're written.
    pub functions: fn(&Lister, SyntaxNode) -> Vec<Function>,
    /// The parts of the function whose item is this node.
    pub parts: for<'t> fn(SyntaxNode<'t>, &Function, &str) -> Option<Parts<'t>>,
    /// One statement, continuing from `exits`.
    pub statement: for<'t> fn(&mut Flow<'_>, SyntaxNode<'t>, Exits) -> Exits,
    /// Calls, and the field naming what they call (`""`: the first named
    /// child).
    pub calls: &'static [(&'static str, &'static str)],
    /// Member accesses, and the field holding the member's name.
    pub members: &'static [(&'static str, &'static str)],
    /// Creating an object, and the field naming its type.
    pub news: &'static [(&'static str, &'static str)],
    /// Assignments, and the field holding what's set (`""`: the first
    /// named child).
    pub sets: &'static [(&'static str, &'static str)],
    /// Wrappers saying nothing themselves: `x;`, `await x`, `(x)`.
    pub transparent: &'static [&'static str],
    /// Calls that end the path, by name: `panic`.
    pub panics: &'static [&'static str],
    pub data: data::Names,
}

/// A function's parts, as its syntax has them.
pub(super) struct Parts<'t> {
    pub parameters: Vec<SyntaxNode<'t>>,
    /// What it returns, as written, if it says.
    pub returns: Option<String>,
    /// Its body: statements in order, or an expression it returns.
    pub body: Option<SyntaxNode<'t>>,
    /// Whether `body` is an expression, whose value it returns.
    pub returns_body: bool,
}

/// Lists a file's functions, with what they're in.
pub(super) struct Lister<'s> {
    pub source: &'s str,
    pub separator: &'static str,
}

impl<'s> Lister<'s> {
    pub fn text(&self, node: SyntaxNode) -> &'s str {
        text(node, self.source)
    }
}

/// Where functions being listed are: the prefix their names take, and the
/// section they're listed under.
#[derive(Clone, Default)]
pub(super) struct Scope {
    owner: Option<String>,
    section: Option<String>,
}

impl Scope {
    /// Inside `owner`, listed under `section` (`class Store`).
    pub fn within(&self, lister: &Lister, owner: &str, section: String) -> Scope {
        Scope {
            owner: Some(match &self.owner {
                Some(outer) => format!("{outer}{}{owner}", lister.separator),
                None => owner.to_string(),
            }),
            section: Some(match &self.section {
                Some(outer) => format!("{outer} › {section}"),
                None => section,
            }),
        }
    }

    /// The function `item`, called `name` there.
    pub fn function(&self, lister: &Lister, item: SyntaxNode, name: SyntaxNode) -> Function {
        self.named(lister, item, name, lister.text(name))
    }

    /// The function `item`, its name at `name` but written `short`.
    pub fn named(
        &self,
        lister: &Lister,
        item: SyntaxNode,
        name: SyntaxNode,
        short: &str,
    ) -> Function {
        Function {
            name: match &self.owner {
                Some(owner) => format!("{owner}{}{short}", lister.separator),
                None => short.to_string(),
            },
            short: short.to_string(),
            section: self.section.clone(),
            range: item.byte_range(),
            name_at: name.start_byte(),
        }
    }
}

/// The node whose bytes are exactly `function.range`.
fn item_at<'t>(root: SyntaxNode<'t>, function: &Function) -> Option<SyntaxNode<'t>> {
    let range = &function.range;
    let found = root.descendant_for_byte_range(range.start, range.end)?;
    std::iter::successors(Some(found), |node| node.parent())
        .find(|node| node.byte_range() == *range)
}

pub(super) fn functions(syntax: &Syntax, root: SyntaxNode, source: &str) -> Vec<Function> {
    let lister = Lister {
        source,
        separator: syntax.separator,
    };
    (syntax.functions)(&lister, root)
}

/// The flow of the function at `function.range`, or `None` if it's no
/// longer there.
pub(super) fn graph(
    syntax: &Syntax,
    root: SyntaxNode,
    source: &str,
    function: &Function,
) -> Option<Graph> {
    let item = item_at(root, function)?;
    let parts = (syntax.parts)(item, function, source)?;
    let mut flow = Flow {
        syntax,
        source,
        graph: Graph::default(),
        breaks: Vec::new(),
    };
    let entry = flow.entry(function, &parts);
    match parts.body {
        Some(body) if parts.returns_body => {
            flow.end(body, "Return", vec![(entry, 0)]);
        }
        Some(body) => {
            flow.block(body, vec![(entry, 0)]);
        }
        None => {}
    }
    data::add(&mut flow.graph, syntax, &parts, source);
    Some(flow.graph)
}

/// The calls to `function` in the file, each with the function making it,
/// matched by name.
pub(super) fn callers_in(
    syntax: &Syntax,
    root: SyntaxNode,
    source: &str,
    function: &Function,
) -> Vec<(String, usize)> {
    let functions = functions(syntax, root, source);
    let mut calls = Vec::new();
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        if let Some(name) = callee_name(syntax, node)
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

/// The child in `field`, or for `""` the first named one.
pub(super) fn field<'t>(node: SyntaxNode<'t>, field: &str) -> Option<SyntaxNode<'t>> {
    if field.is_empty() {
        node.named_child(0)
    } else {
        node.child_by_field_name(field)
    }
}

/// The node naming what `call` calls, if it's a call: `push` in
/// `this.items.push(x)`.
fn callee_name<'t>(syntax: &Syntax, call: SyntaxNode<'t>) -> Option<SyntaxNode<'t>> {
    let &(_, name) = syntax.calls.iter().find(|(kind, _)| *kind == call.kind())?;
    member_name(syntax, field(call, name)?)
}

/// The name at the end of `a.b.c`, `a::c` or `c`.
fn member_name<'t>(syntax: &Syntax, mut node: SyntaxNode<'t>) -> Option<SyntaxNode<'t>> {
    for _ in 0..8 {
        match syntax.members.iter().find(|(kind, _)| *kind == node.kind()) {
            Some(&(_, name)) => node = field(node, name)?,
            None => break,
        }
    }
    (node.named_child_count() == 0).then_some(node)
}

/// A case of a switch: its label, its statements, and whether it runs on
/// into the next case when it doesn't end.
pub(super) struct Case<'t> {
    pub label: String,
    pub statements: Vec<SyntaxNode<'t>>,
    pub falls: bool,
}

/// A handler of a try: its label (`Catch`) and its body.
pub(super) struct Handler<'t> {
    pub label: String,
    pub body: Option<SyntaxNode<'t>>,
}

/// Builds a function's graph, block by block.
pub(super) struct Flow<'s> {
    syntax: &'s Syntax,
    pub source: &'s str,
    graph: Graph,
    /// The `break`s of each loop or switch being built, innermost last,
    /// which continue after it.
    breaks: Vec<Exits>,
}

impl<'s> Flow<'s> {
    pub fn text(&self, node: SyntaxNode) -> &'s str {
        text(node, self.source)
    }

    fn add(&mut self, node: Node, from: &Exits) -> usize {
        let to = self.graph.nodes.len();
        self.graph.nodes.push(node);
        self.graph
            .edges
            .extend(from.iter().map(|&(from, out)| Edge { from, out, to }));
        to
    }

    fn entry(&mut self, function: &Function, parts: &Parts) -> usize {
        let mut code: Vec<String> = parts
            .parameters
            .iter()
            .map(|parameter| one_line(self.text(*parameter)))
            .collect();
        if let Some(returns) = &parts.returns {
            code.push(format!("→ {}", one_line(returns)));
        }
        let end = parts
            .body
            .map_or(function.range.end, |body| body.start_byte());
        self.add(
            Node {
                kind: Kind::Entry,
                title: function.name.clone(),
                code: shorten(code),
                outs: vec![String::new()],
                values: Vec::new(),
                inputs: Vec::new(),
                range: function.range.start..end.max(function.range.start),
                callee: None,
            },
            &Vec::new(),
        )
    }

    /// The statements among `block`'s named children, in order.
    pub fn block(&mut self, block: SyntaxNode, exits: Exits) -> Exits {
        let mut cursor = block.walk();
        let statements: Vec<_> = block.named_children(&mut cursor).collect();
        self.statements(statements, exits)
    }

    pub fn statements(&mut self, statements: Vec<SyntaxNode>, mut exits: Exits) -> Exits {
        for statement in statements {
            if !statement.is_extra() {
                exits = self.statement(statement, exits);
            }
        }
        exits
    }

    /// One statement, as the language reads it.
    pub fn statement(&mut self, statement: SyntaxNode, exits: Exits) -> Exits {
        (self.syntax.statement)(self, statement, exits)
    }

    /// A statement that just runs and goes on: a call, a `new`, an
    /// assignment, or anything else; a call to a panic ends the path.
    pub fn step(&mut self, statement: SyntaxNode, exits: Exits) -> Exits {
        let expression = self.unwrap(statement);
        let (kind, title) = self.describe(expression);
        if kind == Kind::Exit {
            return self.end(statement, &title, exits);
        }
        let callee = self.callee(expression);
        let node = Node {
            kind,
            title,
            code: code_lines(self.text(statement)),
            outs: vec![String::new()],
            values: Vec::new(),
            inputs: Vec::new(),
            range: statement.byte_range(),
            callee,
        };
        vec![(self.add(node, &exits), 0)]
    }

    /// A step titled by `title` from what `inner` would be called: `Defer
    /// done` for Go's `defer done()`.
    pub fn step_named(
        &mut self,
        statement: SyntaxNode,
        inner: SyntaxNode,
        title: impl FnOnce(String) -> String,
        exits: Exits,
    ) -> Exits {
        let expression = self.unwrap(inner);
        let (kind, said) = self.describe(expression);
        let node = Node {
            kind: if kind == Kind::Exit {
                Kind::Statement
            } else {
                kind
            },
            title: title(said),
            code: code_lines(self.text(statement)),
            outs: vec![String::new()],
            values: Vec::new(),
            inputs: Vec::new(),
            range: statement.byte_range(),
            callee: self.callee(expression),
        };
        vec![(self.add(node, &exits), 0)]
    }

    /// A step binding names, `Let x`, which data wires start from;
    /// `value` is what it's set to, for what it calls.
    pub fn bind(
        &mut self,
        statement: SyntaxNode,
        title: String,
        value: Option<SyntaxNode>,
        exits: Exits,
    ) -> Exits {
        let callee = value.and_then(|value| self.callee(self.unwrap(value)));
        let node = Node {
            kind: Kind::Let,
            title,
            code: code_lines(self.text(statement)),
            outs: vec![String::new()],
            values: Vec::new(),
            inputs: Vec::new(),
            range: statement.byte_range(),
            callee,
        };
        vec![(self.add(node, &exits), 0)]
    }

    /// A block that ends the path: `Return`, `Throw`, `Continue`.
    pub fn end(&mut self, statement: SyntaxNode, title: &str, exits: Exits) -> Exits {
        let node = Node {
            kind: Kind::Exit,
            title: title.to_string(),
            code: code_lines(self.text(statement)),
            outs: Vec::new(),
            values: Vec::new(),
            inputs: Vec::new(),
            range: statement.byte_range(),
            callee: None,
        };
        self.add(node, &exits);
        Vec::new()
    }

    /// `break`: on after the innermost loop or switch.
    pub fn exit_loop(&mut self, statement: SyntaxNode, exits: Exits) -> Exits {
        let node = Node {
            kind: Kind::Exit,
            title: "Break".into(),
            code: code_lines(self.text(statement)),
            outs: vec![String::new()],
            values: Vec::new(),
            inputs: Vec::new(),
            range: statement.byte_range(),
            callee: None,
        };
        let at = self.add(node, &exits);
        match self.breaks.last_mut() {
            Some(breaks) => breaks.push((at, 0)),
            // Out of a labelled block: on from there.
            None => return vec![(at, 0)],
        }
        Vec::new()
    }

    /// A block with several outputs: from the start of `statement` to
    /// `header_end`, its code `code`.
    #[allow(clippy::too_many_arguments)]
    pub fn fork(
        &mut self,
        statement: SyntaxNode,
        header_end: Option<usize>,
        kind: Kind,
        title: &str,
        code: &str,
        outs: Vec<String>,
        exits: &Exits,
    ) -> usize {
        let end = header_end.unwrap_or(statement.end_byte());
        self.add(
            Node {
                kind,
                title: title.to_string(),
                code: code_lines(code),
                outs,
                values: Vec::new(),
                inputs: Vec::new(),
                range: statement.start_byte()..end.max(statement.start_byte()),
                callee: None,
            },
            exits,
        )
    }

    /// `if`: True runs `then`, False `otherwise` (which may be another
    /// `if`), both on to what follows.
    pub fn branch(
        &mut self,
        statement: SyntaxNode,
        condition: &str,
        then: Option<SyntaxNode>,
        otherwise: Option<SyntaxNode>,
        exits: Exits,
    ) -> Exits {
        self.branch_with(
            statement,
            condition,
            then,
            exits,
            |flow, from| match otherwise {
                Some(otherwise) => flow.statement(otherwise, from),
                None => from,
            },
        )
    }

    /// `if`, its False side built by `otherwise` from where it starts:
    /// for an `elif` chain kept beside the `if` rather than in it.
    pub fn branch_with(
        &mut self,
        statement: SyntaxNode,
        condition: &str,
        then: Option<SyntaxNode>,
        exits: Exits,
        otherwise: impl FnOnce(&mut Self, Exits) -> Exits,
    ) -> Exits {
        let header = then.map(|then| then.start_byte());
        let at = self.fork(
            statement,
            header,
            Kind::Branch,
            "Branch",
            condition,
            vec!["True".into(), "False".into()],
            &exits,
        );
        let mut after = match then {
            Some(then) => self.statement(then, vec![(at, 0)]),
            None => vec![(at, 0)],
        };
        after.extend(otherwise(self, vec![(at, 1)]));
        after
    }

    /// A loop: its body runs from Body. Returns what goes on from
    /// Completed (when `completes`) and from its `break`s, apart.
    pub fn repeat(
        &mut self,
        statement: SyntaxNode,
        title: &str,
        head: &str,
        body: Option<SyntaxNode>,
        completes: bool,
        exits: Exits,
    ) -> (Exits, Exits) {
        let mut outs = vec!["Body".to_string()];
        if completes {
            outs.push("Completed".into());
        }
        let at = self.fork(
            statement,
            body.map(|body| body.start_byte()),
            Kind::Loop,
            title,
            head,
            outs,
            &exits,
        );
        self.breaks.push(Vec::new());
        if let Some(body) = body {
            self.statement(body, vec![(at, 0)]);
        }
        let breaks = self.breaks.pop().unwrap_or_default();
        let completed = if completes { vec![(at, 1)] } else { Vec::new() };
        (completed, breaks)
    }

    /// A loop whose Completed and `break`s both go on.
    pub fn loop_on(
        &mut self,
        statement: SyntaxNode,
        title: &str,
        head: &str,
        body: Option<SyntaxNode>,
        completes: bool,
        exits: Exits,
    ) -> Exits {
        let (mut after, breaks) = self.repeat(statement, title, head, body, completes, exits);
        after.extend(breaks);
        after
    }

    /// A switch, with an output per case labelled with its value; with no
    /// default, a Default output goes on.
    #[allow(clippy::too_many_arguments)]
    pub fn switch(
        &mut self,
        statement: SyntaxNode,
        header_end: Option<usize>,
        title: &str,
        value: &str,
        cases: Vec<Case>,
        has_default: bool,
        exits: Exits,
    ) -> Exits {
        let mut outs: Vec<String> = cases
            .iter()
            .map(|case| clip(&one_line(&case.label), MAX_PIN_LABEL))
            .collect();
        if !has_default {
            outs.push("Default".into());
        }
        let count = cases.len();
        let at = self.fork(
            statement,
            header_end,
            Kind::Match,
            title,
            value,
            outs,
            &exits,
        );
        self.breaks.push(Vec::new());
        let mut falling: Exits = Vec::new();
        let mut after = Vec::new();
        for (ix, case) in cases.into_iter().enumerate() {
            let mut from = vec![(at, ix)];
            from.append(&mut falling);
            let left = self.statements(case.statements, from);
            if case.falls {
                falling = left;
            } else {
                after.extend(left);
            }
        }
        let mut breaks = self.breaks.pop().unwrap_or_default();
        breaks.extend(after);
        breaks.extend(falling);
        if !has_default {
            breaks.push((at, count));
        }
        breaks
    }

    /// A try: its body from Body, each handler from its own output, an
    /// `else` after the body when it doesn't fail, all on through
    /// `finally`.
    pub fn attempt(
        &mut self,
        statement: SyntaxNode,
        body: Option<SyntaxNode>,
        handlers: Vec<Handler>,
        otherwise: Option<SyntaxNode>,
        finally: Option<SyntaxNode>,
        exits: Exits,
    ) -> Exits {
        let mut outs = vec!["Body".to_string()];
        outs.extend(
            handlers
                .iter()
                .map(|handler| clip(&handler.label, MAX_PIN_LABEL)),
        );
        let at = self.fork(
            statement,
            body.map(|body| body.start_byte()),
            Kind::Branch,
            "Try",
            "",
            outs,
            &exits,
        );
        let mut after = match body {
            Some(body) => self.statement(body, vec![(at, 0)]),
            None => vec![(at, 0)],
        };
        if let Some(otherwise) = otherwise {
            after = self.statement(otherwise, after);
        }
        for (ix, handler) in handlers.into_iter().enumerate() {
            let from = vec![(at, ix + 1)];
            match handler.body {
                Some(body) => after.extend(self.statement(body, from)),
                None => after.extend(from),
            }
        }
        match finally {
            Some(finally) => self.statement(finally, after),
            None => after,
        }
    }

    /// What's inside wrappers that say nothing: `x` in `await (x);`, the
    /// last thing in them (`x` in Swift's `try x`).
    pub fn unwrap<'t>(&self, mut node: SyntaxNode<'t>) -> SyntaxNode<'t> {
        while self.syntax.transparent.contains(&node.kind()) {
            let mut cursor = node.walk();
            let inner = node
                .named_children(&mut cursor)
                .filter(|child| !child.is_extra())
                .last();
            match inner {
                Some(inner) => node = inner,
                None => break,
            }
        }
        node
    }

    /// What's inside a condition's parentheses, as written.
    pub fn condition(&self, node: Option<SyntaxNode>) -> String {
        node.map(|node| self.text(self.unwrap(node)).to_string())
            .unwrap_or_default()
    }

    /// A statement's kind and title: what it calls, makes or sets.
    fn describe(&self, expression: SyntaxNode) -> (Kind, String) {
        let syntax = self.syntax;
        if let Some(name) = callee_name(syntax, expression) {
            let name = self.text(name);
            if syntax.panics.contains(&name) {
                return (Kind::Exit, "Panic".into());
            }
            return (Kind::Call, format!("Call {name}"));
        }
        if syntax
            .calls
            .iter()
            .any(|(kind, _)| *kind == expression.kind())
        {
            return (Kind::Call, "Call".into());
        }
        if let Some(&(_, ty)) = syntax
            .news
            .iter()
            .find(|(kind, _)| *kind == expression.kind())
        {
            let ty = field(expression, ty).map_or("", |ty| self.text(ty));
            return (Kind::Call, format!("New {}", clip(&one_line(ty), 32)));
        }
        if let Some(&(_, target)) = syntax
            .sets
            .iter()
            .find(|(kind, _)| *kind == expression.kind())
        {
            let target = field(expression, target).map_or("", |target| self.text(target));
            return (
                Kind::Statement,
                format!("Set {}", clip(&one_line(target), 32)),
            );
        }
        (Kind::Statement, "Statement".into())
    }

    /// Where the name of what `expression` calls or makes starts.
    fn callee(&self, expression: SyntaxNode) -> Option<usize> {
        let syntax = self.syntax;
        if let Some(name) = callee_name(syntax, expression) {
            return Some(name.start_byte());
        }
        let &(_, ty) = syntax
            .news
            .iter()
            .find(|(kind, _)| *kind == expression.kind())?;
        member_name(syntax, field(expression, ty)?).map(|name| name.start_byte())
    }

    /// The text from `range.start` up to `range.end`, on one line, without
    /// `prefix` and the parentheses around it: a loop's head.
    pub fn head(&self, range: Range<usize>, prefixes: &[&str]) -> String {
        let mut head = self.source.get(range).unwrap_or("").trim();
        for prefix in prefixes {
            head = head.strip_prefix(prefix).unwrap_or(head).trim_start();
        }
        let head = head.trim_end().trim_end_matches(['{', ':']).trim_end();
        let head = match head.strip_prefix('(') {
            Some(inner) => inner.strip_suffix(')').unwrap_or(inner),
            None => head,
        };
        one_line(head)
    }
}

/// A list's text, for a label: `1, 2`.
pub(super) fn label_of(flow: &Flow, nodes: &[SyntaxNode]) -> String {
    nodes
        .iter()
        .map(|node| one_line(flow.text(*node)))
        .collect::<Vec<_>>()
        .join(", ")
}
