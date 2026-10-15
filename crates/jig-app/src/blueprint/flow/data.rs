//! Data wires for every language but Rust: which binding block or
//! parameter each block reads, found by walking the function's names scope
//! by scope, so the latest binding before a use is the one it reads.
//! Bindings without a block of their own (a loop's, a nested function's
//! parameters, a `catch`'s) hide the outer ones but draw no wire. The
//! language says, in `Names`, what its names, scopes and bindings look like.

use std::collections::HashMap;

use tree_sitter::Node as SyntaxNode;

use super::{Parts, Syntax};
use crate::blueprint::data::connect;
use crate::blueprint::{Graph, Kind, text};

/// What names look like in a language.
pub(in crate::blueprint) struct Names {
    /// Nodes that are a name, read where they're used.
    pub names: &'static [&'static str],
    /// Nodes whose names are only seen inside them: blocks.
    pub scopes: &'static [&'static str],
    /// Nodes with no names in them worth wiring: types, labels.
    pub skip: &'static [&'static str],
    /// Children that are a name but not a variable's: (node, field), as a
    /// member's name in `a.b` or a keyword argument's.
    pub not_variables: &'static [(&'static str, &'static str)],
    /// Patterns with a default: (node, the pattern's field, the default's
    /// field). The default is read, the pattern bound.
    pub defaults: &'static [(&'static str, &'static str, &'static str)],
    /// What a node binds, if anything.
    pub binds: for<'t> fn(SyntaxNode<'t>, &str) -> Binds<'t>,
}

/// What a node binds.
pub(in crate::blueprint) enum Binds<'t> {
    Nothing,
    /// A declaration: these patterns, to its own block, after the rest of
    /// it (its values) is read.
    Block(Vec<SyntaxNode<'t>>),
    /// A loop's, a `catch`'s or a nested function's: these patterns, seen
    /// only inside it. They needn't be its own children.
    Inside(Vec<SyntaxNode<'t>>),
}

/// The names in one scope, each with the block that binds it, if any.
type Scope<'s> = HashMap<&'s str, Option<usize>>;

struct Resolver<'s> {
    source: &'s str,
    names: &'s Names,
    /// Each binding block, by where its statement starts.
    lets: HashMap<usize, usize>,
    scopes: Vec<Scope<'s>>,
    /// The names each block binds, in the order they're bound.
    bound: Vec<(usize, &'s str)>,
    /// Every use of a block's binding: (binding block, name, where).
    uses: Vec<(usize, &'s str, usize)>,
    /// Patterns being bound, which aren't read.
    hidden: Vec<usize>,
}

/// Fill in `graph`'s data wires, and its blocks' data pins.
pub(super) fn add(graph: &mut Graph, syntax: &Syntax, parts: &Parts, source: &str) {
    let mut resolver = Resolver {
        source,
        names: &syntax.data,
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
        hidden: Vec::new(),
    };
    for parameter in &parts.parameters {
        resolver.bind(*parameter, Some(0));
    }
    if let Some(body) = parts.body {
        resolver.walk(body);
    }
    connect(graph, &resolver.bound, &resolver.uses);
}

impl<'s> Resolver<'s> {
    fn scoped(&mut self, inside: impl FnOnce(&mut Self)) {
        self.scopes.push(Scope::new());
        inside(self);
        self.scopes.pop();
    }

    /// `node`'s named children but those in `except`, and those that are
    /// a name but not a variable's.
    fn children<'t>(&self, node: SyntaxNode<'t>, except: &[SyntaxNode<'t>]) -> Vec<SyntaxNode<'t>> {
        let skipped: Vec<SyntaxNode> = self
            .names
            .not_variables
            .iter()
            .filter(|(kind, _)| *kind == node.kind())
            .flat_map(|(_, field)| {
                let mut cursor = node.walk();
                node.children_by_field_name(field, &mut cursor)
                    .collect::<Vec<_>>()
            })
            .collect();
        let mut cursor = node.walk();
        node.named_children(&mut cursor)
            .filter(|child| !except.contains(child) && !skipped.contains(child))
            .collect()
    }

    fn walk(&mut self, node: SyntaxNode) {
        let kind = node.kind();
        if self.names.skip.contains(&kind) || self.hidden.contains(&node.id()) {
            return;
        }
        if self.names.names.contains(&kind) {
            self.read(node);
            return;
        }
        match (self.names.binds)(node, self.source) {
            Binds::Block(patterns) => {
                // Its values can't see what it binds.
                let hidden = self.hidden.len();
                self.hidden
                    .extend(patterns.iter().map(|pattern| pattern.id()));
                for child in self.children(node, &patterns) {
                    self.walk(child);
                }
                self.hidden.truncate(hidden);
                let binder = self.binder(node);
                for pattern in patterns {
                    self.bind(pattern, binder);
                }
            }
            Binds::Inside(patterns) => self.scoped(|this| {
                for pattern in &patterns {
                    this.bind(*pattern, None);
                }
                let hidden = this.hidden.len();
                this.hidden
                    .extend(patterns.iter().map(|pattern| pattern.id()));
                for child in this.children(node, &patterns) {
                    this.walk(child);
                }
                this.hidden.truncate(hidden);
            }),
            Binds::Nothing if self.names.scopes.contains(&kind) => self.scoped(|this| {
                for child in this.children(node, &[]) {
                    this.walk(child);
                }
            }),
            Binds::Nothing => {
                for child in self.children(node, &[]) {
                    self.walk(child);
                }
            }
        }
    }

    /// The block binding what `node` binds: the one its statement starts.
    fn binder(&self, node: SyntaxNode) -> Option<usize> {
        std::iter::successors(Some(node), |node| node.parent())
            .take(3)
            .find_map(|node| self.lets.get(&node.start_byte()).copied())
    }

    /// Every name `pattern` binds, to `binder`.
    fn bind(&mut self, pattern: SyntaxNode, binder: Option<usize>) {
        let kind = pattern.kind();
        if self.names.skip.contains(&kind) {
            return;
        }
        if self.names.names.contains(&kind) {
            let name = text(pattern, self.source);
            if let Some(scope) = self.scopes.last_mut() {
                scope.insert(name, binder);
            }
            if let Some(binder) = binder {
                self.bound.push((binder, name));
            }
            return;
        }
        if let Some(&(_, bound, default)) = self.names.defaults.iter().find(|(k, _, _)| *k == kind)
        {
            if let Some(default) = pattern.child_by_field_name(default) {
                self.walk(default);
            }
            if let Some(bound) = pattern.child_by_field_name(bound) {
                self.bind(bound, binder);
            }
            return;
        }
        for child in self.children(pattern, &[]) {
            self.bind(child, binder);
        }
    }

    fn read(&mut self, name: SyntaxNode) {
        let text = text(name, self.source);
        let binding = self
            .scopes
            .iter()
            .rev()
            .find_map(|scope| scope.get(text).copied());
        if let Some(Some(binder)) = binding {
            self.uses.push((binder, text, name.start_byte()));
        }
    }
}
