//! Data wires in JavaScript and TypeScript: which `const`, `let` or `var`
//! block or parameter each block reads, found by walking the function's
//! names scope by scope, as in Rust. Bindings without a block of their own
//! (a loop's, a nested function's parameters, a `catch`'s) hide the outer
//! ones but draw no wire.

use std::collections::HashMap;

use tree_sitter::Node as SyntaxNode;

use crate::blueprint::data::connect;
use crate::blueprint::{Graph, Kind, text};

/// The names in one scope, each with the block that binds it, if any.
type Scope<'s> = HashMap<&'s str, Option<usize>>;

struct Resolver<'s> {
    source: &'s str,
    /// Each declaration block, by where its statement starts.
    lets: HashMap<usize, usize>,
    scopes: Vec<Scope<'s>>,
    /// The names each block binds, in the order they're bound.
    bound: Vec<(usize, &'s str)>,
    /// Every use of a block's binding: (binding block, name, where).
    uses: Vec<(usize, &'s str, usize)>,
}

/// Fill in `graph`'s data wires, and its blocks' data pins, from the
/// function it was read from.
pub(super) fn add(graph: &mut Graph, function: SyntaxNode, source: &str) {
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
    resolver.parameters(function, Some(0));
    if let Some(body) = function.child_by_field_name("body") {
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

    /// A function's parameters, `(a, b)` or `a =>`, bound to `binder`.
    fn parameters(&mut self, function: SyntaxNode, binder: Option<usize>) {
        if let Some(parameters) = function.child_by_field_name("parameters") {
            self.bind(parameters, binder);
        }
        if let Some(parameter) = function.child_by_field_name("parameter") {
            self.bind(parameter, binder);
        }
    }

    fn walk(&mut self, node: SyntaxNode) {
        match node.kind() {
            "identifier" | "shorthand_property_identifier" => self.read(node),
            "statement_block" | "for_statement" | "switch_body" => {
                self.scoped(|this| this.walk_children(node))
            }
            "lexical_declaration" | "variable_declaration" => {
                let binder = self.lets.get(&node.start_byte()).copied();
                let mut cursor = node.walk();
                for declarator in node.named_children(&mut cursor) {
                    // Its value can't see what it binds.
                    self.walk_field(declarator, "value");
                    if let Some(name) = declarator.child_by_field_name("name") {
                        self.bind(name, binder);
                    }
                }
            }
            "for_in_statement" => {
                self.walk_field(node, "right");
                self.scoped(|this| {
                    match (
                        node.child_by_field_name("kind"),
                        node.child_by_field_name("left"),
                    ) {
                        (Some(_), Some(left)) => this.bind(left, None),
                        (None, Some(left)) => this.walk(left),
                        _ => {}
                    }
                    this.walk_field(node, "body");
                });
            }
            "arrow_function"
            | "function_expression"
            | "function"
            | "generator_function"
            | "function_declaration"
            | "generator_function_declaration"
            | "method_definition" => self.scoped(|this| {
                this.parameters(node, None);
                this.walk_field(node, "body");
            }),
            "catch_clause" => self.scoped(|this| {
                if let Some(parameter) = node.child_by_field_name("parameter") {
                    this.bind(parameter, None);
                }
                this.walk_field(node, "body");
            }),
            // Types name types, not variables.
            "type_annotation"
            | "type_arguments"
            | "type_parameters"
            | "type_alias_declaration"
            | "interface_declaration"
            | "statement_identifier" => {}
            _ => self.walk_children(node),
        }
    }

    /// Every name `pattern` binds, to `binder`.
    fn bind(&mut self, pattern: SyntaxNode, binder: Option<usize>) {
        match pattern.kind() {
            "identifier" | "shorthand_property_identifier_pattern" => {
                self.add_binding(text(pattern, self.source), binder);
            }
            "required_parameter" | "optional_parameter" => {
                if let Some(pattern) = pattern.child_by_field_name("pattern") {
                    self.bind(pattern, binder);
                }
                self.walk_field(pattern, "value");
            }
            // A default reads what's around it.
            "assignment_pattern" | "object_assignment_pattern" => {
                self.walk_field(pattern, "right");
                if let Some(left) = pattern.child_by_field_name("left") {
                    self.bind(left, binder);
                }
            }
            "pair_pattern" => {
                if let Some(value) = pattern.child_by_field_name("value") {
                    self.bind(value, binder);
                }
            }
            "type_annotation" | "this" | "property_identifier" => {}
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
