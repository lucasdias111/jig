//! Data wires: which `let` block or parameter each block reads, found by
//! walking the function's names scope by scope, so the latest binding
//! before a use is the one it reads. Bindings without a block of their own
//! (a loop's, a closure's, a match arm's, `if let`, a `let` inside an
//! expression) hide the outer ones but draw no wire.

use std::collections::HashMap;

use tree_sitter::Node as SyntaxNode;

use super::{Data, Graph, Kind, text};

/// The names in one scope, each with the block that binds it, if any.
type Scope<'s> = HashMap<&'s str, Option<usize>>;

struct Resolver<'s> {
    source: &'s str,
    /// Each `let` block, by where its statement starts.
    lets: HashMap<usize, usize>,
    scopes: Vec<Scope<'s>>,
    /// The names each block binds, in the order they're bound.
    bound: Vec<(usize, &'s str)>,
    /// Every use of a block's binding: (binding block, name, where).
    uses: Vec<(usize, &'s str, usize)>,
}

/// Fill in `graph`'s data wires, and its blocks' data pins, from the
/// function `item` it was read from.
pub(super) fn add(graph: &mut Graph, item: SyntaxNode, source: &str) {
    let mut resolver = Resolver {
        source,
        lets: graph
            .nodes
            .iter()
            .enumerate()
            .filter(|(_, node)| node.kind == Kind::Let)
            .map(|(ix, node)| (node.range.start, ix))
            .collect(),
        scopes: vec![Scope::new()],
        bound: Vec::new(),
        uses: Vec::new(),
    };
    if let Some(parameters) = item.child_by_field_name("parameters") {
        let mut cursor = parameters.walk();
        for parameter in parameters.named_children(&mut cursor) {
            if let Some(pattern) = parameter.child_by_field_name("pattern") {
                resolver.bind(pattern, Some(0));
            }
        }
    }
    if let Some(body) = item.child_by_field_name("body") {
        resolver.walk(body);
    }
    connect(graph, &resolver.bound, &resolver.uses);
}

/// A wire for each block reading each binding, the binding's pins in the
/// order it binds them and the reader's in the order it reads them.
pub(super) fn connect(graph: &mut Graph, bound: &[(usize, &str)], uses: &[(usize, &str, usize)]) {
    let mut reads: Vec<(usize, &str, usize)> = Vec::new();
    for &(from, name, at) in uses {
        let Some(to) = graph
            .nodes
            .iter()
            .position(|node| node.kind != Kind::Entry && node.range.contains(&at))
        else {
            continue;
        };
        if to != from && !reads.contains(&(from, name, to)) {
            reads.push((from, name, to));
        }
    }
    for &(from, name) in bound {
        let read = reads.iter().any(|&(f, n, _)| f == from && n == name);
        let values = &mut graph.nodes[from].values;
        if read && !values.iter().any(|value| value == name) {
            values.push(name.to_string());
        }
    }
    // An input per binding read, so two of the same name stay apart.
    for (from, name, to) in reads {
        let Some(value) = graph.nodes[from]
            .values
            .iter()
            .position(|value| value == name)
        else {
            continue;
        };
        let inputs = &mut graph.nodes[to].inputs;
        inputs.push(name.to_string());
        let input = inputs.len() - 1;
        graph.data.push(Data {
            from,
            value,
            to,
            input,
        });
    }
}

impl<'s> Resolver<'s> {
    fn scoped(&mut self, inside: impl FnOnce(&mut Self)) {
        self.scopes.push(Scope::new());
        inside(self);
        self.scopes.pop();
    }

    fn walk_field(&mut self, node: SyntaxNode, field: &str) {
        if let Some(child) = node.child_by_field_name(field) {
            self.walk(child);
        }
    }

    fn walk_children(&mut self, node: SyntaxNode) {
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            self.walk(child);
        }
    }

    fn walk(&mut self, node: SyntaxNode) {
        match node.kind() {
            "identifier" => self.read(node),
            "block" => self.scoped(|this| this.walk_children(node)),
            "let_declaration" => {
                // Its value can't see what it binds.
                self.walk_field(node, "value");
                self.walk_field(node, "alternative");
                let binder = self.lets.get(&node.start_byte()).copied();
                if let Some(pattern) = node.child_by_field_name("pattern") {
                    self.bind(pattern, binder);
                }
            }
            "let_condition" => {
                self.walk_field(node, "value");
                if let Some(pattern) = node.child_by_field_name("pattern") {
                    self.bind(pattern, None);
                }
            }
            // `if let` and `while let` bind only for their own body.
            "if_expression" => {
                self.scoped(|this| {
                    this.walk_field(node, "condition");
                    this.walk_field(node, "consequence");
                });
                self.walk_field(node, "alternative");
            }
            "while_expression" => self.scoped(|this| {
                this.walk_field(node, "condition");
                this.walk_field(node, "body");
            }),
            "for_expression" => {
                self.walk_field(node, "value");
                self.scoped(|this| {
                    if let Some(pattern) = node.child_by_field_name("pattern") {
                        this.bind(pattern, None);
                    }
                    this.walk_field(node, "body");
                });
            }
            "closure_expression" => self.scoped(|this| {
                if let Some(parameters) = node.child_by_field_name("parameters") {
                    this.bind(parameters, None);
                }
                this.walk_field(node, "body");
            }),
            "match_arm" => self.scoped(|this| {
                if let Some(pattern) = node.child_by_field_name("pattern") {
                    let guard = pattern.child_by_field_name("condition");
                    let mut cursor = pattern.walk();
                    for child in pattern.named_children(&mut cursor) {
                        if Some(child) != guard {
                            this.bind(child, None);
                        }
                    }
                    if let Some(guard) = guard {
                        this.walk(guard);
                    }
                }
                this.walk_field(node, "value");
            }),
            "macro_invocation" => {
                let mut cursor = node.walk();
                for child in node.named_children(&mut cursor) {
                    if child.kind() == "token_tree" {
                        self.walk_tokens(child);
                    }
                }
            }
            // Paths name items, not variables; nested items can't see
            // the function's.
            "scoped_identifier" | "label" | "lifetime" | "attribute_item" => {}
            kind if super::is_declaration(kind) => {}
            _ => self.walk_children(node),
        }
    }

    /// A macro's arguments, which are only tokens: a name reads a
    /// variable unless it's a field (`.name`) or in a path (`a::name`).
    fn walk_tokens(&mut self, tree: SyntaxNode) {
        let mut cursor = tree.walk();
        let tokens: Vec<_> = tree.children(&mut cursor).collect();
        for (ix, token) in tokens.iter().enumerate() {
            let before = ix.checked_sub(1).map(|ix| tokens[ix].kind());
            let after = tokens.get(ix + 1).map(|token| token.kind());
            match token.kind() {
                "identifier" if !matches!(before, Some("." | "::")) && after != Some("::") => {
                    self.read(*token)
                }
                "token_tree" => self.walk_tokens(*token),
                _ => {}
            }
        }
    }

    /// Every name `pattern` binds, to `binder`.
    fn bind(&mut self, pattern: SyntaxNode, binder: Option<usize>) {
        match pattern.kind() {
            // `None` and `MAX` in a pattern are constants, not bindings.
            "identifier" => {
                let name = text(pattern, self.source);
                if !name.starts_with(char::is_uppercase) {
                    self.add_binding(name, binder);
                }
            }
            "shorthand_field_identifier" => {
                self.add_binding(text(pattern, self.source), binder);
            }
            "parameter" => {
                if let Some(pattern) = pattern.child_by_field_name("pattern") {
                    self.bind(pattern, binder);
                }
            }
            "tuple_struct_pattern" | "struct_pattern" => {
                let path = pattern.child_by_field_name("type");
                let mut cursor = pattern.walk();
                for child in pattern.named_children(&mut cursor) {
                    if Some(child) != path {
                        self.bind(child, binder);
                    }
                }
            }
            "scoped_identifier" | "type_identifier" | "field_identifier" => {}
            _ => {
                let mut cursor = pattern.walk();
                for child in pattern.named_children(&mut cursor) {
                    self.bind(child, binder);
                }
            }
        }
    }

    fn add_binding(&mut self, name: &'s str, binder: Option<usize>) {
        if let Some(scope) = self.scopes.last_mut() {
            scope.insert(name, binder);
        }
        if let Some(binder) = binder {
            self.bound.push((binder, name));
        }
    }

    fn read(&mut self, identifier: SyntaxNode) {
        let name = text(identifier, self.source);
        let binding = self
            .scopes
            .iter()
            .rev()
            .find_map(|scope| scope.get(name).copied());
        if let Some(Some(binder)) = binding {
            self.uses.push((binder, name, identifier.start_byte()));
        }
    }
}
