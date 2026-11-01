//! The types a file declares, and its fields and variables.

use super::*;

/// Kinds of type declarations, and what they're called.
pub(super) const TYPE_KINDS: &[(&str, &str)] = &[
    ("class_declaration", "class"),
    ("abstract_class_declaration", "class"),
    ("interface_declaration", "interface"),
    ("enum_declaration", "enum"),
    ("record_declaration", "record"),
    ("struct_item", "struct"),
    ("trait_item", "trait"),
    ("enum_item", "enum"),
];

pub(super) fn collect_types(node: SyntaxNode, source: &str, found: &mut Vec<TypeDecl>) {
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
pub(super) const FIELD_KINDS: &[&str] = &[
    "field_declaration",
    "public_field_definition",
    "field_definition",
    "property_declaration",
    "container_field",
    "expression_statement",
];

/// Declarations of variables at the top of a file.
pub(super) const GLOBAL_KINDS: &[&str] = &[
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
pub(super) fn variables(
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

pub(super) fn is_function_like(kind: &str) -> bool {
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
