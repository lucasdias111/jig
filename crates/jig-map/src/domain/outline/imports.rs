//! A file's package and the paths its imports name.

use super::*;

pub(super) fn package(root: SyntaxNode, source: &str) -> Option<String> {
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

pub(super) fn imports(root: SyntaxNode, source: &str, lang: Lang) -> Vec<Import> {
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
