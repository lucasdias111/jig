//! What the outline needs from each language's syntax: its functions, in
//! the order they're written and named as they're called (`Type::name`,
//! `Store.save`), each function's parameters and what it returns, and
//! which of its nodes are calls. Rust is read here; every other language
//! by the tables and hooks its `Syntax` in `lang` gives.

use std::ops::Range;

use tree_sitter::{Node as SyntaxNode, Parser, Tree};

use super::lang;

/// A language the Map reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lang {
    Rust,
    JavaScript,
    TypeScript,
    Tsx,
    Python,
    Go,
    Java,
    C,
    Cpp,
    Swift,
    Zig,
}

impl Lang {
    fn grammar(self) -> tree_sitter::Language {
        match self {
            Lang::Rust => tree_sitter_rust::LANGUAGE.into(),
            Lang::JavaScript => tree_sitter_javascript::LANGUAGE.into(),
            Lang::TypeScript => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            Lang::Tsx => tree_sitter_typescript::LANGUAGE_TSX.into(),
            Lang::Python => tree_sitter_python::LANGUAGE.into(),
            Lang::Go => tree_sitter_go::LANGUAGE.into(),
            Lang::Java => tree_sitter_java::LANGUAGE.into(),
            Lang::C => tree_sitter_c::LANGUAGE.into(),
            Lang::Cpp => tree_sitter_cpp::LANGUAGE.into(),
            Lang::Swift => tree_sitter_swift::LANGUAGE.into(),
            Lang::Zig => tree_sitter_zig::LANGUAGE.into(),
        }
    }
}

/// A function in the file, named as it's called: `Type::name` in an impl.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Function {
    /// As it's called: `Type::name`.
    pub name: String,
    /// Just its own name.
    pub short: String,
    /// What it's in: `Workspace` for an impl, `Display for Workspace` for
    /// a trait's, `trait Named`, `mod tests`, joined by ` › ` when they
    /// nest. `None` at the top of the file.
    pub section: Option<String>,
    /// The whole item, signature and body, as byte offsets.
    pub range: Range<usize>,
    /// Where its name starts, to ask a language server about it.
    pub name_at: usize,
}

pub(super) fn parse(source: &str, lang: Lang) -> Option<Tree> {
    let mut parser = Parser::new();
    parser.set_language(&lang.grammar()).ok()?;
    parser.parse(source, None)
}

pub(super) fn text<'s>(node: SyntaxNode, source: &'s str) -> &'s str {
    source.get(node.byte_range()).unwrap_or("")
}

/// What a language's syntax looks like, as far as the outline goes.
pub(super) struct Syntax {
    /// Between an owner's name and its function's: `Store.save`.
    pub separator: &'static str,
    /// The file's functions, in the order they're written.
    pub functions: fn(&Lister, SyntaxNode) -> Vec<Function>,
    /// The parts of the function whose item is this node.
    pub parts: for<'t> fn(SyntaxNode<'t>, &Function, &str) -> Option<Parts<'t>>,
    /// Calls, and the field naming what they call (`""`: the first named
    /// child).
    pub calls: &'static [(&'static str, &'static str)],
    /// Member accesses, and the field holding the member's name.
    pub members: &'static [(&'static str, &'static str)],
}

/// A function's parts, as its syntax has them.
pub(super) struct Parts<'t> {
    pub parameters: Vec<SyntaxNode<'t>>,
    /// What it returns, as written, if it says.
    pub returns: Option<String>,
    /// Its body: statements in order, or an expression it returns.
    pub body: Option<SyntaxNode<'t>>,
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

/// The file's functions with bodies, in the order they're written,
/// including those in impls, traits' default methods and modules (in
/// other languages, classes, namespaces and the like).
#[cfg(test)]
pub fn functions(source: &str, lang: Lang) -> Vec<Function> {
    let Some(tree) = parse(source, lang) else {
        return Vec::new();
    };
    listed(tree.root_node(), source, lang)
}

/// The functions under `root`, as `functions` lists them.
pub(super) fn listed(root: SyntaxNode, source: &str, lang: Lang) -> Vec<Function> {
    match lang::syntax(lang) {
        Some(syntax) => {
            let lister = Lister {
                source,
                separator: syntax.separator,
            };
            (syntax.functions)(&lister, root)
        }
        None => {
            let mut found = Vec::new();
            collect_rust(root, source, &RustScope::default(), &mut found);
            found
        }
    }
}

/// The node whose bytes are exactly `function.range`.
pub(super) fn item_at<'t>(root: SyntaxNode<'t>, function: &Function) -> Option<SyntaxNode<'t>> {
    let range = &function.range;
    let found = root.descendant_for_byte_range(range.start, range.end)?;
    std::iter::successors(Some(found), |node| node.parent())
        .find(|node| node.byte_range() == *range)
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
pub(super) fn callee_name<'t>(syntax: &Syntax, call: SyntaxNode<'t>) -> Option<SyntaxNode<'t>> {
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

/// The name in a Rust callee: `push` in `self.items.push`, `new` in
/// `Vec::new`.
pub(super) fn callee_name_node(callee: SyntaxNode) -> Option<SyntaxNode> {
    match callee.kind() {
        "field_expression" => callee.child_by_field_name("field"),
        "generic_function" => callee_name_node(callee.child_by_field_name("function")?),
        "scoped_identifier" => callee.child_by_field_name("name"),
        _ => Some(callee),
    }
}

/// Where Rust functions being collected are: the prefix their names take,
/// and the section they're listed under.
#[derive(Clone, Default)]
struct RustScope {
    owner: Option<String>,
    section: Option<String>,
}

impl RustScope {
    fn within(&self, owner: Option<String>, section: String) -> RustScope {
        RustScope {
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

fn collect_rust(node: SyntaxNode, source: &str, scope: &RustScope, found: &mut Vec<Function>) {
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
            collect_rust(body, source, &inner, found);
        }
    }
}

/// `Workspace` from `Workspace<'a, T>`.
fn base_type(name: &str) -> String {
    name.split('<').next().unwrap_or(name).trim().to_string()
}

/// The innermost function whose item contains `offset`.
#[cfg(test)]
pub fn containing(functions: &[Function], offset: usize) -> Option<usize> {
    functions
        .iter()
        .enumerate()
        .filter(|(_, function)| function.range.contains(&offset))
        .min_by_key(|(_, function)| function.range.len())
        .map(|(ix, _)| ix)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_rust_functions_with_their_owners() {
        let source = "fn free() {}\n\
                      struct A;\n\
                      impl<T> Wrapper<T> { fn get(&self) {} }\n\
                      trait Named { fn name(&self) -> String { String::new() } fn bare(&self); }\n\
                      mod inner { fn hidden() {} }\n";
        let names: Vec<_> = functions(source, Lang::Rust)
            .into_iter()
            .map(|f| f.name)
            .collect();
        assert_eq!(
            names,
            ["free", "Wrapper::get", "Named::name", "inner::hidden"]
        );
    }

    #[test]
    fn finds_the_innermost_function_around_an_offset() {
        let source = "fn a() {}\nfn b() { let x = 1; }\n";
        let found = functions(source, Lang::Rust);
        assert_eq!(containing(&found, source.find("let").unwrap()), Some(1));
        assert_eq!(containing(&found, 9), None);
    }
}
