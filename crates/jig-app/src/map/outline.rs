//! A file read for the Map, in one parse: its functions with their
//! contracts (parameters in, what they return out) and the calls they
//! make, the types it declares and what they implement, what it imports,
//! and the types names are declared with, so a call on a variable can be
//! followed to its type's method. Functions without a body (an
//! interface's or a trait's) are listed too, marked abstract, since calls
//! through them go to them. Read with tree-sitter, so it works without a
//! language server and on code that doesn't build.

mod lang;
pub mod syntax;

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

impl Visibility {
    /// As UML writes it: `+`, `#`, `~`, `−`.
    pub fn symbol(self) -> &'static str {
        match self {
            Visibility::Public => "+",
            Visibility::Protected => "#",
            Visibility::Internal => "~",
            Visibility::Private => "−",
        }
    }

    pub fn word(self) -> &'static str {
        match self {
            Visibility::Public => "public",
            Visibility::Protected => "protected",
            Visibility::Internal => "internal",
            Visibility::Private => "private",
        }
    }
}

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

impl Parameter {
    /// `name: Type`, or just the name.
    pub fn label(&self) -> String {
        match &self.ty {
            Some(ty) => format!("{}: {ty}", self.name),
            None => self.name.clone(),
        }
    }
}

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

fn package(root: SyntaxNode, source: &str) -> Option<String> {
    let mut cursor = root.walk();
    let declaration = root
        .named_children(&mut cursor)
        .find(|child| child.kind() == "package_declaration")?;
    let mut cursor = declaration.walk();
    let name = declaration
        .named_children(&mut cursor)
        .find(|child| child.kind().contains("identifier"))?;
    Some(text(name, source).to_string())
}

fn imports(root: SyntaxNode, source: &str, lang: Lang) -> Vec<Import> {
    let mut found = Vec::new();
    let mut cursor = root.walk();
    for child in root.named_children(&mut cursor) {
        match child.kind() {
            // Java: `import a.b.C;`, `import static a.b.C.m;`, `a.b.*`.
            "import_declaration" => {
                let written = text(child, source)
                    .trim_start_matches("import")
                    .trim()
                    .trim_start_matches("static")
                    .trim()
                    .trim_end_matches(';')
                    .trim()
                    .to_string();
                let wildcard = written.ends_with(".*");
                let path = written.trim_end_matches(".*").to_string();
                let name = path.rsplit('.').next().unwrap_or(&path).to_string();
                found.push(Import {
                    names: if wildcard {
                        Vec::new()
                    } else {
                        vec![(name.clone(), name)]
                    },
                    path,
                    wildcard,
                });
            }
            // TypeScript and JavaScript: `import a, { b as c } from './x'`.
            "import_statement" => {
                let Some(from) = child.child_by_field_name("source") else {
                    continue;
                };
                let path = text(from, source)
                    .trim_matches(['"', '\'', '`'])
                    .to_string();
                let mut import = Import {
                    path,
                    ..Import::default()
                };
                let mut stack = vec![child];
                while let Some(node) = stack.pop() {
                    match node.kind() {
                        "import_clause" => {
                            let mut cursor = node.walk();
                            for part in node.named_children(&mut cursor) {
                                if part.kind() == "identifier" {
                                    let name = text(part, source).to_string();
                                    import.names.push((name, "default".into()));
                                } else {
                                    stack.push(part);
                                }
                            }
                        }
                        "import_specifier" => {
                            let name = node.child_by_field_name("name");
                            let alias = node.child_by_field_name("alias").or(name);
                            if let (Some(name), Some(alias)) = (name, alias) {
                                import
                                    .names
                                    .push((text(alias, source).into(), text(name, source).into()));
                            }
                        }
                        "namespace_import" => {
                            let mut cursor = node.walk();
                            if let Some(name) = node
                                .named_children(&mut cursor)
                                .find(|part| part.kind() == "identifier")
                            {
                                let name = text(name, source).to_string();
                                import.names.push((name.clone(), name));
                                import.wildcard = true;
                            }
                        }
                        _ => {
                            let mut cursor = node.walk();
                            stack.extend(node.named_children(&mut cursor));
                        }
                    }
                }
                found.push(import);
            }
            "use_declaration" if lang == Lang::Rust => {
                let path = text(child, source)
                    .trim_start_matches("pub")
                    .trim()
                    .trim_start_matches("use")
                    .trim()
                    .trim_end_matches(';')
                    .to_string();
                let name = path
                    .rsplit("::")
                    .next()
                    .unwrap_or(&path)
                    .trim_matches(['{', '}', ' '])
                    .to_string();
                found.push(Import {
                    wildcard: path.ends_with('*'),
                    names: vec![(name.clone(), name)],
                    path,
                });
            }
            _ => {}
        }
    }
    found
}

/// Kinds of type declarations, and what they're called.
const TYPE_KINDS: &[(&str, &str)] = &[
    ("class_declaration", "class"),
    ("abstract_class_declaration", "class"),
    ("interface_declaration", "interface"),
    ("enum_declaration", "enum"),
    ("record_declaration", "record"),
    ("struct_item", "struct"),
    ("trait_item", "trait"),
    ("enum_item", "enum"),
];

fn collect_types(node: SyntaxNode, source: &str, found: &mut Vec<TypeDecl>) {
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if let Some(&(_, kind)) = TYPE_KINDS.iter().find(|(k, _)| *k == child.kind())
            && let Some(name) = child.child_by_field_name("name")
        {
            let mut implements = Vec::new();
            for field in ["superclass", "interfaces", "extends_interfaces"] {
                if let Some(list) = child.child_by_field_name(field) {
                    type_names(list, source, &mut implements);
                }
            }
            // TypeScript: `class A extends B implements C`.
            let mut parts = child.walk();
            for part in child.named_children(&mut parts) {
                if matches!(part.kind(), "class_heritage" | "extends_type_clause") {
                    type_names(part, source, &mut implements);
                }
            }
            let mut fields = Vec::new();
            if let Some(body) = child.child_by_field_name("body") {
                let mut members = body.walk();
                for member in body.named_children(&mut members) {
                    match member.kind() {
                        "field_declaration" | "public_field_definition" | "field_definition" => {
                            typed_names(member, source, &mut fields)
                        }
                        // `constructor(private repo: Repo)`.
                        "method_definition" => {
                            if let Some(parameters) = member.child_by_field_name("parameters") {
                                typed_names(parameters, source, &mut fields);
                            }
                        }
                        _ => {}
                    }
                }
            }
            let mut components = Vec::new();
            if let Some(parameters) = child.child_by_field_name("parameters") {
                let mut cursor = parameters.walk();
                for parameter in parameters.named_children(&mut cursor) {
                    if let (Some(name), Some(ty)) = (
                        parameter.child_by_field_name("name"),
                        parameter.child_by_field_name("type"),
                    ) {
                        components.push((
                            text(name, source).to_string(),
                            one_line_type(text(ty, source)),
                        ));
                    }
                }
            }
            found.push(TypeDecl {
                name: text(name, source).to_string(),
                kind,
                implements,
                fields,
                components,
                range: child.byte_range(),
                purpose: doc_before(child, source),
            });
        }
        // Rust: `impl Trait for Type`.
        if child.kind() == "impl_item"
            && let (Some(implemented), Some(ty)) = (
                child.child_by_field_name("trait"),
                child.child_by_field_name("type"),
            )
        {
            let ty = simple_type(text(ty, source));
            match found.iter_mut().find(|t| t.name == ty) {
                Some(decl) => decl.implements.push(simple_type(text(implemented, source))),
                None => found.push(TypeDecl {
                    name: ty,
                    kind: "struct",
                    implements: vec![simple_type(text(implemented, source))],
                    fields: Vec::new(),
                    components: Vec::new(),
                    range: child.byte_range(),
                    purpose: None,
                }),
            }
        }
        if !child.kind().ends_with("_declaration") || child.child_count() > 0 {
            collect_types(child, source, found);
        }
    }
}

/// Declarations of fields in a type's body.
const FIELD_KINDS: &[&str] = &[
    "field_declaration",
    "public_field_definition",
    "field_definition",
    "property_declaration",
    "container_field",
    "expression_statement",
];
/// Declarations of variables at the top of a file.
const GLOBAL_KINDS: &[&str] = &[
    "lexical_declaration",
    "variable_declaration",
    "const_item",
    "static_item",
    "var_declaration",
    "const_declaration",
    "declaration",
    "expression_statement",
    "property_declaration",
];

/// The fields of the types under `node` and, at the top (`top`), the
/// variables it declares: never what's inside a function.
fn variables(
    node: SyntaxNode,
    owner: Option<&str>,
    top: bool,
    source: &str,
    lang: Lang,
    found: &mut Vec<Variable>,
) {
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        let kind = child.kind();
        if is_function_like(kind) {
            continue;
        }
        if let Some(name) = type_name(child, source) {
            variables(child, Some(&name), false, source, lang, found);
            continue;
        }
        let declares = match owner {
            Some(_) => FIELD_KINDS.contains(&kind),
            None => top && GLOBAL_KINDS.contains(&kind),
        };
        if declares {
            for (name, ty) in declared(child, source) {
                let before = source
                    .get(child.start_byte()..name.start_byte())
                    .unwrap_or("");
                let named = text(name, source).to_string();
                found.push(Variable {
                    owner: owner.map(str::to_string),
                    visibility: visibility(before, &named, lang),
                    name: named,
                    ty,
                    at: name.start_byte(),
                });
            }
            continue;
        }
        // Through bodies, and `export`s at the top.
        let through = owner.is_some()
            || kind.ends_with("body")
            || matches!(
                kind,
                "export_statement" | "declaration_list" | "type_declaration"
            );
        if through {
            variables(child, owner, top, source, lang, found);
        }
    }
}

fn is_function_like(kind: &str) -> bool {
    [
        "function",
        "method",
        "constructor",
        "lambda",
        "closure",
        "arrow",
        "init_declaration",
    ]
    .iter()
    .any(|part| kind.contains(part))
        || kind == "impl_item"
}

/// The name of the type `node` declares, if it declares one: a class,
/// struct, interface and the like, a Go `type_spec`, Zig's `const X =
/// struct {…}`.
fn type_name(node: SyntaxNode, source: &str) -> Option<String> {
    let kind = node.kind();
    let declares = TYPE_KINDS.iter().any(|(k, _)| *k == kind)
        || matches!(
            kind,
            "class_definition"
                | "class_specifier"
                | "struct_specifier"
                | "type_spec"
                | "protocol_declaration"
                | "object_declaration"
        );
    if declares {
        let name = node.child_by_field_name("name")?;
        // A C `struct x;` with no body declares nothing to list.
        return Some(text(name, source).to_string());
    }
    if kind == "variable_declaration" {
        let mut cursor = node.walk();
        let children: Vec<SyntaxNode> = node.named_children(&mut cursor).collect();
        if children.iter().any(|child| {
            matches!(
                child.kind(),
                "struct_declaration" | "enum_declaration" | "union_declaration"
            )
        }) {
            let name = children.iter().find(|child| child.kind() == "identifier")?;
            return Some(text(*name, source).to_string());
        }
    }
    None
}

/// The names a declaration declares, with their types as written.
fn declared<'t>(node: SyntaxNode<'t>, source: &str) -> Vec<(SyntaxNode<'t>, Option<String>)> {
    let ty = |node: SyntaxNode| {
        node.child_by_field_name("type")
            .map(|ty| one_line_type(text(ty, source)))
    };
    let mut found = Vec::new();
    let mut cursor = node.walk();
    let declarators: Vec<SyntaxNode> = node
        .children_by_field_name("declarator", &mut cursor)
        .collect();
    if !declarators.is_empty() {
        for declarator in declarators {
            if let Some(name) = declarator_name(declarator) {
                found.push((name, ty(declarator).or_else(|| ty(node))));
            }
        }
    } else if let Some(name) = node
        .child_by_field_name("name")
        .or_else(|| node.child_by_field_name("property"))
    {
        let mut cursor = node.walk();
        let names: Vec<SyntaxNode> = node.children_by_field_name("name", &mut cursor).collect();
        for name in if names.is_empty() { vec![name] } else { names } {
            if let Some(name) = declarator_name(name) {
                found.push((name, ty(node)));
            }
        }
    } else {
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            match child.kind() {
                // Python: `x = 1`, `x: int = 1`.
                "assignment" => {
                    if let Some(left) = child
                        .child_by_field_name("left")
                        .filter(|left| left.kind() == "identifier")
                    {
                        found.push((left, ty(child)));
                    }
                }
                "variable_declarator" | "var_spec" | "const_spec" | "init_declarator" => {
                    let value = child.child_by_field_name("value").map(|v| v.kind());
                    let is_function = value
                        .is_some_and(|value| is_function_like(value) || value.contains("class"));
                    if !is_function {
                        found.extend(declared(child, source));
                    }
                }
                _ => {}
            }
        }
    }
    found.retain(|(name, _)| is_name(text(*name, source)));
    found
}

/// The name a declarator declares, through pointers and arrays; `None`
/// for a function's.
fn declarator_name(node: SyntaxNode) -> Option<SyntaxNode> {
    let kind = node.kind();
    if kind.contains("function") {
        return None;
    }
    if kind.ends_with("identifier") {
        return Some(node);
    }
    if let Some(inner) = node
        .child_by_field_name("declarator")
        .or_else(|| node.child_by_field_name("name"))
        .or_else(|| node.child_by_field_name("pattern"))
    {
        return declarator_name(inner);
    }
    let mut cursor = node.walk();
    let first = node.named_children(&mut cursor).next()?;
    first.kind().ends_with("identifier").then_some(first)
}

fn one_line_type(written: &str) -> String {
    written
        .trim()
        .trim_start_matches(':')
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// The type names under `node`, by simple name.
fn type_names(node: SyntaxNode, source: &str, found: &mut Vec<String>) {
    let mut stack = vec![node];
    while let Some(node) = stack.pop() {
        if matches!(node.kind(), "type_identifier" | "identifier") {
            found.push(text(node, source).to_string());
            continue;
        }
        if node.kind() == "type_arguments" {
            continue;
        }
        let mut cursor = node.walk();
        let children: Vec<_> = node.named_children(&mut cursor).collect();
        stack.extend(children.into_iter().rev());
    }
}

/// `Repo` from `: Repo<T>`, `&mut Repo`, `a.b.Repo[]`, `crate::x::Repo`.
pub fn simple_type(written: &str) -> String {
    let written = written
        .trim()
        .trim_start_matches(':')
        .trim()
        .trim_start_matches('&')
        .trim_start_matches("mut ")
        .trim_start_matches("readonly ")
        .trim();
    let base = written
        .split(['<', '[', '?', ' ', '|'])
        .next()
        .unwrap_or(written);
    base.rsplit(['.', ':']).next().unwrap_or(base).to_string()
}

/// Names declared with a type under `node`: parameters, locals, fields.
pub(super) fn typed_names(node: SyntaxNode, source: &str, found: &mut Vec<(String, String)>) {
    let mut stack = vec![node];
    while let Some(node) = stack.pop() {
        if let Some(ty) = node.child_by_field_name("type") {
            let ty = simple_type(text(ty, source));
            let mut names = Vec::new();
            for field in ["name", "pattern"] {
                if let Some(name) = node.child_by_field_name(field) {
                    names.push(name);
                }
            }
            let mut cursor = node.walk();
            names.extend(
                node.children_by_field_name("declarator", &mut cursor)
                    .filter_map(|d| d.child_by_field_name("name").or(Some(d))),
            );
            for name in names {
                let name = text(name, source);
                if is_name(name) && !ty.is_empty() {
                    found.push((name.to_string(), ty.clone()));
                }
            }
        }
        let mut cursor = node.walk();
        stack.extend(node.named_children(&mut cursor));
    }
}

fn is_name(text: &str) -> bool {
    !text.is_empty()
        && text
            .chars()
            .all(|c| c.is_alphanumeric() || c == '_' || c == '$')
}

/// A function's parameters, what it returns, and its body.
fn contract<'t>(
    syntax: Option<&Syntax>,
    item: SyntaxNode<'t>,
    function: &Function,
    source: &str,
) -> (Vec<Parameter>, Option<String>, Option<SyntaxNode<'t>>) {
    let (nodes, returns, body) = match syntax {
        Some(syntax) => match (syntax.parts)(item, function, source) {
            Some(parts) => (parts.parameters, parts.returns, parts.body),
            None => (Vec::new(), None, None),
        },
        None => {
            let nodes = item
                .child_by_field_name("parameters")
                .map(|list| {
                    let mut cursor = list.walk();
                    list.named_children(&mut cursor)
                        .filter(|p| !p.is_extra())
                        .collect()
                })
                .unwrap_or_default();
            let returns = item
                .child_by_field_name("return_type")
                .map(|r| text(r, source).to_string());
            (nodes, returns, item.child_by_field_name("body"))
        }
    };
    let parameters = nodes
        .into_iter()
        .filter(|node| !node.is_extra())
        .map(|node| parameter(node, source))
        .collect();
    let returns = returns
        .map(|r| r.trim().trim_start_matches(':').trim().to_string())
        .filter(|r| !r.is_empty() && r != "void" && r != "()");
    (parameters, returns, body)
}

fn parameter(node: SyntaxNode, source: &str) -> Parameter {
    let name = node
        .child_by_field_name("name")
        .or_else(|| node.child_by_field_name("pattern"))
        .map(|name| text(name, source).to_string());
    let ty = node.child_by_field_name("type").map(|ty| {
        text(ty, source)
            .trim()
            .trim_start_matches(':')
            .trim()
            .to_string()
    });
    match name {
        Some(name) => Parameter { name, ty },
        None => Parameter {
            name: text(node, source)
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" "),
            ty: None,
        },
    }
}

/// The calls under `item`, and the functions it hands on.
fn calls(
    syntax: Option<&Syntax>,
    item: SyntaxNode,
    source: &str,
    parameters: &[&str],
) -> Vec<Call> {
    let mut found = Vec::new();
    let mut stack = vec![item];
    while let Some(node) = stack.pop() {
        let name = match syntax {
            Some(syntax) => callee_name(syntax, node),
            None if node.kind() == "call_expression" => node
                .child_by_field_name("function")
                .and_then(callee_name_node),
            None => None,
        };
        if let Some(name) = name {
            let lead = source
                .get(node.start_byte()..name.start_byte())
                .unwrap_or("");
            let qualifier = qualifier(lead);
            let called = text(name, source).to_string();
            found.push(Call {
                through_parameter: qualifier.is_none() && parameters.contains(&called.as_str()),
                name: called,
                qualifier,
                at: name.start_byte(),
                bound: bound(node, source),
                arguments: arguments(node, source),
                reference: false,
            });
            // What it's handed that may be a function.
            if let Some(list) = node.child_by_field_name("arguments") {
                let mut cursor = list.walk();
                for argument in list.named_children(&mut cursor) {
                    if let Some(reference) = handed(argument, source) {
                        found.push(reference);
                    }
                }
            }
        } else if node.kind() == "method_reference" {
            // Java: `this::handle`, `Repo::save`.
            let written = text(node, source);
            if let Some((on, method)) = written.rsplit_once("::") {
                found.push(Call {
                    name: method.trim().to_string(),
                    qualifier: qualifier(&format!("{on}.")),
                    at: node.start_byte() + written.len() - method.len(),
                    reference: true,
                    ..Call::default()
                });
            }
        }
        let mut cursor = node.walk();
        let children: Vec<_> = node.named_children(&mut cursor).collect();
        stack.extend(children.into_iter().rev());
    }
    found.sort_by_key(|call| call.at);
    found
}

/// What a call is made on, from the text before its name: `this.repo.`
/// is `repo`; `this.` none; `a().` an expression (`""`).
fn qualifier(lead: &str) -> Option<String> {
    let lead = lead.trim_end_matches(['.', ':', '-', '>', '?', ' ', '\n', '\t']);
    if lead.is_empty() {
        return None;
    }
    let last = lead
        .rsplit(['.', ':', ' ', '(', '\n'])
        .next()
        .unwrap_or(lead);
    match last {
        "this" | "self" | "Self" | "super" => None,
        _ if lead.ends_with(')') || lead.ends_with(']') || !is_name(last) => Some(String::new()),
        _ => Some(last.to_string()),
    }
}

/// The name a call's result is bound to, if it's bound to one.
fn bound(call: SyntaxNode, source: &str) -> Option<String> {
    let mut node = call;
    for _ in 0..4 {
        let parent = node.parent()?;
        let value = parent
            .child_by_field_name("value")
            .or_else(|| parent.child_by_field_name("right"));
        if value.is_some_and(|value| value.byte_range().contains(&call.start_byte())) {
            let name = parent
                .child_by_field_name("name")
                .or_else(|| parent.child_by_field_name("pattern"))
                .or_else(|| parent.child_by_field_name("left"))?;
            let name = text(name, source).trim_start_matches("mut ").to_string();
            return is_name(&name).then_some(name);
        }
        if parent.kind().ends_with("statement") || parent.kind().ends_with("block") {
            return None;
        }
        node = parent;
    }
    None
}

/// The names a call is handed.
fn arguments(call: SyntaxNode, source: &str) -> Vec<String> {
    let Some(list) = call.child_by_field_name("arguments") else {
        return Vec::new();
    };
    let mut names: Vec<String> = Vec::new();
    let mut stack = vec![list];
    while let Some(node) = stack.pop() {
        if node.kind() == "identifier" {
            let name = text(node, source).to_string();
            if !names.contains(&name) {
                names.push(name);
            }
            continue;
        }
        let mut cursor = node.walk();
        let children: Vec<_> = node.named_children(&mut cursor).collect();
        stack.extend(children.into_iter().rev());
    }
    names
}

/// An argument that may be a function handed on: a name, or a member
/// (`this.handle`).
fn handed(argument: SyntaxNode, source: &str) -> Option<Call> {
    if !matches!(
        argument.kind(),
        "identifier" | "member_expression" | "field_expression" | "scoped_identifier"
    ) {
        return None;
    }
    let written = text(argument, source);
    let (lead, name) = match written.rfind(['.', ':']) {
        Some(at) => (&written[..=at], &written[at + 1..]),
        None => ("", written),
    };
    is_name(name).then(|| Call {
        name: name.to_string(),
        qualifier: qualifier(lead),
        at: argument.start_byte() + lead.len(),
        reference: true,
        ..Call::default()
    })
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

/// Where each line starts, to tell which line a byte is on.
struct Lines(Vec<usize>);

impl Lines {
    fn of(source: &str) -> Lines {
        let mut starts = vec![0];
        starts.extend(source.match_indices('\n').map(|(at, _)| at + 1));
        Lines(starts)
    }

    /// The line (0-based) byte `at` is on.
    fn at(&self, at: usize) -> u32 {
        (self.0.partition_point(|&start| start <= at) - 1) as u32
    }
}

/// The first statement in `body`, or the body itself when it's an
/// expression or empty.
fn first_statement(body: SyntaxNode) -> SyntaxNode {
    let mut cursor = body.walk();
    let first = body
        .named_children(&mut cursor)
        .find(|child| !child.is_extra() && !child.kind().contains("comment"));
    match first {
        // A block's statements are the ones that run.
        Some(first) if body.kind().contains("block") || body.kind().ends_with("body") => first,
        _ => body,
    }
}

/// Comments, as tree-sitter's grammars name them.
fn is_comment(node: SyntaxNode) -> bool {
    node.kind().contains("comment")
}

/// What the doc comment right before `node` says it's for, in a line.
fn doc_before(node: SyntaxNode, source: &str) -> Option<String> {
    let mut comments = Vec::new();
    let mut at = node.prev_sibling();
    // Attributes and decorators sit between a type and its doc.
    while let Some(previous) = at {
        if is_comment(previous) {
            comments.push(text(previous, source));
        } else if !matches!(previous.kind(), "attribute_item" | "decorator" | "annotation") {
            break;
        }
        at = previous.prev_sibling();
    }
    comments.reverse();
    purpose_of(&comments.join("\n"))
}

/// What the file is for: its leading doc comment (`//!`, a docstring, a
/// comment before Go's `package`), unless it's a licence.
fn file_purpose(root: SyntaxNode, source: &str, lang: Lang) -> Option<String> {
    let mut cursor = root.walk();
    let children: Vec<SyntaxNode> = root.named_children(&mut cursor).collect();
    if lang == Lang::Python
        && let Some(first) = children.iter().find(|child| !is_comment(**child))
        && first.kind() == "expression_statement"
        && first.named_child(0).is_some_and(|node| node.kind() == "string")
    {
        return purpose_of(text(*first, source));
    }
    let leading: Vec<&str> = children
        .iter()
        .take_while(|child| is_comment(**child))
        .map(|child| text(*child, source))
        // Rust's file doc is `//!`; its `///` belongs to what follows.
        .filter(|comment| lang != Lang::Rust || comment.starts_with("//!"))
        .collect();
    purpose_of(&leading.join("\n"))
}

/// A comment's first sentence, its markers taken off, on one line; none
/// for an empty one or a licence.
fn purpose_of(comment: &str) -> Option<String> {
    let mut words = Vec::new();
    for line in comment.lines() {
        let line = line
            .trim()
            .trim_start_matches("/**")
            .trim_start_matches("/*")
            .trim_end_matches("*/")
            .trim_start_matches(['/', '!', '*', '#'])
            .trim_matches(['"', '\''])
            .trim();
        if line.is_empty() {
            if !words.is_empty() {
                break;
            }
            continue;
        }
        if line.starts_with('@') {
            break;
        }
        words.extend(line.split_whitespace());
    }
    let text = words.join(" ");
    let lower = text.to_lowercase();
    if text.is_empty()
        || ["copyright", "license", "licence", "spdx"]
            .iter()
            .any(|word| lower.contains(word))
    {
        return None;
    }
    // Its first sentence, unless that's all it says.
    let end = text
        .match_indices(". ")
        .map(|(at, _)| at + 1)
        .find(|&at| at > 12)
        .unwrap_or(text.len());
    Some(text[..end].trim_end_matches(':').to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    const JAVA: &str = "package org.margin.bugs;\n\nimport org.margin.users.UserService;\nimport org.slf4j.Logger;\n\npublic class BugService implements Reporter {\n    private final UserService users;\n    private Logger log;\n\n    public Bug create(String title, long userId) {\n        User user = users.find(userId);\n        Bug bug = save(title, user);\n        log.info(\"made\");\n        items.forEach(this::notify);\n        return bug;\n    }\n\n    private Bug save(String title, User user) { return null; }\n    private void notify(Bug bug) {}\n}\n\ninterface Reporter {\n    Bug create(String title, long userId);\n}\n";

    #[test]
    fn java_functions_contracts_and_calls() {
        let outline = outline(JAVA, Lang::Java);
        assert_eq!(outline.package.as_deref(), Some("org.margin.bugs"));
        assert_eq!(outline.imports[0].path, "org.margin.users.UserService");
        let class = &outline.types[0];
        assert_eq!(class.name, "BugService");
        assert_eq!(class.implements, ["Reporter"]);
        assert!(
            class
                .fields
                .contains(&("users".into(), "UserService".into()))
        );
        let create = &outline.functions[0];
        assert_eq!(create.function.name, "BugService.create");
        assert_eq!(create.owner.as_deref(), Some("BugService"));
        let parameters: Vec<String> = create.parameters.iter().map(Parameter::label).collect();
        assert_eq!(parameters, ["title: String", "userId: long"]);
        assert_eq!(create.returns.as_deref(), Some("Bug"));
        // Names handed on are kept as possible callbacks; only those naming
        // a function count, which the index decides.
        let calls: Vec<(&str, Option<&str>, Option<&str>, bool)> = create
            .calls
            .iter()
            .filter(|c| !c.reference || c.name == "notify")
            .map(|c| {
                (
                    c.name.as_str(),
                    c.qualifier.as_deref(),
                    c.bound.as_deref(),
                    c.reference,
                )
            })
            .collect();
        assert_eq!(
            calls,
            [
                ("find", Some("users"), Some("user"), false),
                ("save", None, Some("bug"), false),
                ("info", Some("log"), None, false),
                ("forEach", Some("items"), None, false),
                ("notify", None, None, true),
            ]
        );
        let save = create.calls.iter().find(|c| c.name == "save").unwrap();
        assert_eq!(save.arguments, ["title", "user"]);
        let abstract_create = outline
            .functions
            .iter()
            .find(|f| f.is_abstract)
            .expect("the interface's method");
        assert_eq!(abstract_create.function.name, "Reporter.create");
        assert_eq!(abstract_create.owner.as_deref(), Some("Reporter"));
    }

    const TS: &str = "import { Room } from './types.js'\nimport * as mediasoup from 'mediasoup'\nimport helper, { join as enter } from '../helpers'\n\nexport async function getRoom(rooms: Map<string, Room>, id: string, done: () => void): Promise<Room> {\n    const router = await mediasoup.createRouter()\n    const room = makeRoom(router, id)\n    enter(room)\n    done()\n    return room\n}\n\nfunction makeRoom(router: Router, id: string): Room { return { router, id } }\n\nclass Hub implements Notifier {\n    constructor(private store: Store) {}\n    send(text: string): void { this.store.save(text) }\n}\n\ninterface Notifier { send(text: string): void }\n";

    #[test]
    fn typescript_imports_contracts_and_calls() {
        let outline = outline(TS, Lang::TypeScript);
        assert_eq!(outline.imports[0].path, "./types.js");
        assert_eq!(
            outline.imports[0].names,
            [("Room".to_string(), "Room".to_string())]
        );
        assert!(outline.imports[1].wildcard);
        assert_eq!(
            outline.imports[2].names,
            [
                ("helper".to_string(), "default".to_string()),
                ("enter".to_string(), "join".to_string())
            ]
        );
        let get = &outline.functions[0];
        assert_eq!(get.function.name, "getRoom");
        assert_eq!(get.returns.as_deref(), Some("Promise<Room>"));
        assert_eq!(get.parameters[0].label(), "rooms: Map<string, Room>");
        let calls: Vec<(&str, Option<&str>, bool)> = get
            .calls
            .iter()
            .filter(|c| !c.reference)
            .map(|c| (c.name.as_str(), c.qualifier.as_deref(), c.through_parameter))
            .collect();
        assert_eq!(
            calls,
            [
                ("createRouter", Some("mediasoup"), false),
                ("makeRoom", None, false),
                ("enter", None, false),
                ("done", None, true),
            ]
        );
        let hub = outline.types.iter().find(|t| t.name == "Hub").unwrap();
        assert_eq!(hub.implements, ["Notifier"]);
        assert!(hub.fields.contains(&("store".into(), "Store".into())));
        let send = outline
            .functions
            .iter()
            .find(|f| f.function.name == "Hub.send")
            .unwrap();
        assert_eq!(send.calls[0].qualifier.as_deref(), Some("store"));
        assert!(
            outline
                .functions
                .iter()
                .any(|f| f.is_abstract && f.function.name == "Notifier.send")
        );
    }

    #[test]
    fn fields_and_variables_for_the_overview() {
        let fields = |source: &str, lang: Lang| -> Vec<String> {
            outline(source, lang)
                .variables
                .iter()
                .map(|v| match (&v.owner, &v.ty) {
                    (Some(owner), Some(ty)) => format!("{owner}.{}: {ty}", v.name),
                    (Some(owner), None) => format!("{owner}.{}", v.name),
                    (None, Some(ty)) => format!("{}: {ty}", v.name),
                    (None, None) => v.name.clone(),
                })
                .collect()
        };
        assert_eq!(
            fields(JAVA, Lang::Java),
            ["BugService.users: UserService", "BugService.log: Logger"]
        );
        assert_eq!(
            fields(
                "const LIMIT = 3\nexport let count: number = 0\nconst go = () => {}\nclass Hub { store: Store; run() { let local = 1 } }\n",
                Lang::TypeScript
            ),
            ["LIMIT", "count: number", "Hub.store: Store"]
        );
        assert_eq!(
            fields(
                "LIMIT = 3\n\nclass Hub:\n    size: int = 0\n    def run(self):\n        local = 1\n",
                Lang::Python
            ),
            ["LIMIT", "Hub.size: int"]
        );
        assert_eq!(
            fields(
                "package main\n\nvar count int\n\ntype Hub struct {\n\tstore Store\n}\n\nfunc f() { x := 1 }\n",
                Lang::Go
            ),
            ["count: int", "Hub.store: Store"]
        );
        assert_eq!(
            fields(
                "static LIMIT: u8 = 3;\nstruct Hub { store: Store }\nfn f() { let x = 1; }\n",
                Lang::Rust
            ),
            ["LIMIT: u8", "Hub.store: Store"]
        );
        assert_eq!(
            fields(
                "int count = 0;\nstruct hub { int size; };\nvoid f(void) { int x = 1; }\nint g(void);\n",
                Lang::C
            ),
            ["count: int", "hub.size: int"]
        );
    }

    #[test]
    fn rust_calls_and_impls() {
        let source = "use crate::store::Store;\n\nstruct Hub { store: Store }\n\nimpl Notifier for Hub {\n    fn send(&self, text: &str) -> bool {\n        let saved = self.store.save(text);\n        log(saved)\n    }\n}\n\ntrait Notifier { fn send(&self, text: &str) -> bool; }\n";
        let outline = outline(source, Lang::Rust);
        let hub = outline.types.iter().find(|t| t.name == "Hub").unwrap();
        assert_eq!(hub.implements, ["Notifier"]);
        let send = &outline.functions[0];
        assert_eq!(send.function.name, "Hub::send");
        assert_eq!(send.owner.as_deref(), Some("Hub"));
        assert_eq!(send.calls[0].name, "save");
        assert_eq!(send.calls[0].bound.as_deref(), Some("saved"));
        assert!(
            outline
                .functions
                .iter()
                .any(|f| f.is_abstract && f.function.short == "send")
        );
    }

    #[test]
    fn visibility_as_written_or_as_the_language_implies() {
        use Visibility::*;
        let java = outline(JAVA, Lang::Java);
        let of = |name: &str| {
            java.functions
                .iter()
                .find(|f| f.function.short == name && f.owner.as_deref() == Some("BugService"))
                .and_then(|f| f.visibility)
        };
        assert_eq!(of("create"), Some(Public));
        assert_eq!(of("save"), Some(Private));
        let users = java.variables.iter().find(|v| v.name == "users").unwrap();
        assert_eq!(users.visibility, Some(Private));
        let rust = outline(
            "pub fn a() {}\nfn b() {}\npub(crate) fn c() {}\n",
            Lang::Rust,
        );
        let seen: Vec<_> = rust.functions.iter().map(|f| f.visibility).collect();
        assert_eq!(seen, [Some(Public), Some(Private), Some(Internal)]);
        assert_eq!(visibility("", "_hidden", Lang::Python), Some(Private));
        assert_eq!(visibility("", "Serve", Lang::Go), Some(Public));
        assert_eq!(visibility("", "x", Lang::TypeScript), None);
    }
}
