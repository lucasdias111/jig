//! A file read for the Map, in one parse: its functions with their
//! contracts (parameters in, what they return out) and the calls they
//! make, the types it declares and what they implement, what it imports,
//! and the types names are declared with, so a call on a variable can be
//! followed to its type's method. Functions without a body (an
//! interface's or a trait's) are listed too, marked abstract, since calls
//! through them go to them. Read with tree-sitter, so it works without a
//! language server and on code that doesn't build.

mod calls;
mod imports;
mod lang;
mod purposes;
pub mod syntax;
mod type_names;
mod types;
use calls::*;
use imports::*;
use purposes::*;
pub use type_names::*;
use types::*;

use std::ops::Range;

use tree_sitter::Node as SyntaxNode;

pub use syntax::{Function, Lang};
use syntax::{Syntax, callee_name, callee_name_node, item_at, listed, parse, text};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Outline {
    /// What the file is for, in a line: its doc comment's first sentence.
    pub purpose: Option<String>,
    /// A Java file's `package`.
    pub package: Option<String>,
    pub imports: Vec<Import>,
    pub types: Vec<TypeDecl>,
    pub functions: Vec<FunctionOutline>,
    /// Its types' fields and its own variables, for its overview.
    pub variables: Vec<Variable>,
}

/// A field of a type, or a variable declared at the top of a file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Variable {
    /// The type it's a field of, by simple name; `None` at the top.
    pub owner: Option<String>,
    pub name: String,
    pub ty: Option<String>,
    /// Where its name is.
    pub at: usize,
    pub visibility: Option<Visibility>,
}

/// Who can see a function or field, as written or as its language
/// implies; `None` where neither says.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Visibility {
    Public,
    Protected,
    /// Its package's, crate's or module's.
    Internal,
    Private,
}

impl Visibility {}

/// The visibility of a declaration whose text before its name is
/// `before`, named `name`, in `lang`: its keywords, else its language's
/// convention (Rust and Zig private without `pub`, Go by the name's
/// case, Python's leading underscore, a JavaScript `#name`).
pub fn visibility(before: &str, name: &str, lang: Lang) -> Option<Visibility> {
    let words: Vec<&str> = before
        .split(|c: char| !(c.is_alphanumeric() || c == '_'))
        .filter(|w| !w.is_empty())
        .collect();
    let has = |word: &str| words.contains(&word);
    if has("private") || has("fileprivate") {
        return Some(Visibility::Private);
    }
    if has("protected") {
        return Some(Visibility::Protected);
    }
    if has("internal") || before.contains("pub(crate)") || before.contains("pub(super)") {
        return Some(Visibility::Internal);
    }
    if has("public") || has("pub") || has("export") || has("open") {
        return Some(Visibility::Public);
    }
    if name.starts_with('#') {
        return Some(Visibility::Private);
    }
    match lang {
        Lang::Rust | Lang::Zig => Some(Visibility::Private),
        Lang::Go => Some(if name.starts_with(|c: char| c.is_uppercase()) {
            Visibility::Public
        } else {
            Visibility::Private
        }),
        Lang::Python if name.starts_with('_') && !name.ends_with("__") => Some(Visibility::Private),
        Lang::Python => Some(Visibility::Public),
        _ => None,
    }
}

/// One import, as written: `org.margin.Bug`, `./helpers`, `crate::x`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Import {
    pub path: String,
    /// The names it brings in: (as used here, as exported there).
    pub names: Vec<(String, String)>,
    /// `a.b.*`, or a namespace import (`* as ns`, its name in `names`).
    pub wildcard: bool,
}

/// A class, interface, enum, record, struct or trait.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TypeDecl {
    pub name: String,
    pub kind: &'static str,
    /// What it extends and implements, by simple name.
    pub implements: Vec<String>,
    /// Its fields' names and types.
    pub fields: Vec<(String, String)>,
    /// A record's components, in order: what it's made with.
    pub components: Vec<(String, String)>,
    pub range: Range<usize>,
    /// What it's for, in a line: its doc comment's first sentence.
    pub purpose: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FunctionOutline {
    pub function: Function,
    /// The type it's a method of, by simple name.
    pub owner: Option<String>,
    pub parameters: Vec<Parameter>,
    pub returns: Option<String>,
    /// Declared without a body: an interface's or a trait's.
    pub is_abstract: bool,
    pub calls: Vec<Call>,
    /// The names its parameters and locals are declared with, and their
    /// types (simple names).
    pub locals: Vec<(String, String)>,
    /// Its text, hashed, to tell whether it changed.
    pub hash: u64,
    pub visibility: Option<Visibility>,
    /// The lines it spans (0-based), from its first to past its last.
    pub lines: Range<u32>,
    /// The line its body's first statement is on, which running it
    /// reaches first; `None` without a body.
    pub body_line: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Parameter {
    pub name: String,
    pub ty: Option<String>,
}

#[cfg(any(test, feature = "testing"))]
impl Parameter {
    /// `name: Type`, or just the name.
    pub fn label(&self) -> String {
        match &self.ty {
            Some(ty) => format!("{}: {ty}", self.name),
            None => self.name.clone(),
        }
    }
}

impl Parameter {}

/// A call a function makes, or a function it hands on (a callback).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Call {
    pub name: String,
    /// What it's called on, its last part: `repo` in `this.repo.save()`.
    /// `Some("")` when that's an expression with no name (`a().b()`);
    /// `None` when unqualified or on `this`/`self`.
    pub qualifier: Option<String>,
    pub at: usize,
    /// The name its result is bound to: `user` in `let user = find()`.
    pub bound: Option<String>,
    /// The names it's handed.
    pub arguments: Vec<String>,
    /// A function handed on rather than called: `this::handle`.
    pub reference: bool,
    /// A call of one of the function's own parameters: whatever it was
    /// handed, which nothing here can know.
    pub through_parameter: bool,
}

/// `source` read as the graph needs it.
pub fn outline(source: &str, lang: Lang) -> Outline {
    let Some(tree) = parse(source, lang) else {
        return Outline::default();
    };
    let root = tree.root_node();
    let mut outline = Outline {
        purpose: file_purpose(root, source, lang),
        package: package(root, source),
        imports: imports(root, source, lang),
        types: Vec::new(),
        functions: Vec::new(),
        variables: Vec::new(),
    };
    collect_types(root, source, &mut outline.types);
    variables(root, None, true, source, lang, &mut outline.variables);
    outline.variables.sort_by_key(|variable| variable.at);
    let separator = if lang == Lang::Rust { "::" } else { "." };
    let syntax = lang::syntax(lang);
    let lines = Lines::of(source);
    for function in listed(root, source, lang) {
        let item = match syntax {
            Some(_) => item_at(root, &function),
            None => rust_item(root, &function),
        };
        let Some(item) = item else {
            continue;
        };
        let (parameters, returns, body) = contract(syntax, item, &function, source);
        let mut locals = Vec::new();
        typed_names(item, source, &mut locals);
        let names: Vec<&str> = parameters.iter().map(|p| p.name.as_str()).collect();
        let calls = calls(syntax, item, source, &names);
        let hash = hash(text(item, source));
        let before = source
            .get(item.start_byte().min(function.name_at)..function.name_at)
            .unwrap_or("");
        let visibility = visibility(before, &function.short, lang);
        outline.functions.push(FunctionOutline {
            hash,
            visibility,
            owner: owner(&function.name, separator),
            function,
            parameters,
            returns,
            is_abstract: false,
            calls,
            locals,
            lines: lines.at(item.start_byte())..lines.at(item.end_byte()) + 1,
            body_line: body.map(|body| lines.at(first_statement(body).start_byte())),
        });
    }
    abstract_functions(root, source, separator, &lines, &mut outline.functions);
    outline
}

fn hash(text: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut hasher);
    hasher.finish()
}

/// The type `name` (`Outer.Inner.method`) is a method of: `Inner`.
fn owner(name: &str, separator: &str) -> Option<String> {
    let (owner, _) = name.rsplit_once(separator)?;
    Some(owner.rsplit(separator).next().unwrap_or(owner).to_string())
}

fn rust_item<'t>(root: SyntaxNode<'t>, function: &Function) -> Option<SyntaxNode<'t>> {
    let found = root.descendant_for_byte_range(function.range.start, function.range.end)?;
    std::iter::successors(Some(found), |node| node.parent())
        .find(|node| node.kind() == "function_item")
}

/// Methods declared without a body: in Java interfaces and abstract
/// classes, TypeScript interfaces and abstract classes, Rust traits.
fn abstract_functions(
    root: SyntaxNode,
    source: &str,
    separator: &str,
    lines: &Lines,
    found: &mut Vec<FunctionOutline>,
) {
    let mut stack = vec![(root, Vec::<String>::new())];
    while let Some((node, owners)) = stack.pop() {
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            let mut inner = owners.clone();
            if TYPE_KINDS.iter().any(|(kind, _)| *kind == child.kind())
                && let Some(name) = child.child_by_field_name("name")
            {
                inner.push(text(name, source).to_string());
            }
            let bodiless = match child.kind() {
                "method_declaration" => child.child_by_field_name("body").is_none(),
                "method_signature" | "abstract_method_signature" | "function_signature_item" => {
                    true
                }
                _ => false,
            };
            if bodiless
                && let Some(owner) = owners.last()
                && let Some(name) = child.child_by_field_name("name")
            {
                let short = text(name, source).to_string();
                let parameters = child
                    .child_by_field_name("parameters")
                    .map(|list| {
                        let mut cursor = list.walk();
                        list.named_children(&mut cursor)
                            .filter(|p| !p.is_extra())
                            .map(|p| parameter(p, source))
                            .collect()
                    })
                    .unwrap_or_default();
                let returns = ["type", "return_type"]
                    .iter()
                    .find_map(|field| child.child_by_field_name(field))
                    .map(|r| {
                        text(r, source)
                            .trim()
                            .trim_start_matches(':')
                            .trim()
                            .to_string()
                    })
                    .filter(|r| !r.is_empty() && r != "void");
                let before = source
                    .get(child.start_byte()..name.start_byte())
                    .unwrap_or("");
                found.push(FunctionOutline {
                    hash: hash(text(child, source)),
                    // An interface's methods are as visible as it is.
                    visibility: visibility(before, &short, Lang::Java).or(Some(Visibility::Public)),
                    function: Function {
                        name: format!("{}{separator}{short}", owners.join(separator)),
                        short,
                        section: Some(format!("interface {owner}")),
                        range: child.byte_range(),
                        name_at: name.start_byte(),
                    },
                    owner: Some(owner.clone()),
                    parameters,
                    returns,
                    is_abstract: true,
                    calls: Vec::new(),
                    locals: Vec::new(),
                    lines: lines.at(child.start_byte())..lines.at(child.end_byte()) + 1,
                    body_line: None,
                });
                continue;
            }
            stack.push((child, inner));
        }
    }
    found.sort_by_key(|f| f.function.range.start);
}

#[cfg(test)]
mod tests;
