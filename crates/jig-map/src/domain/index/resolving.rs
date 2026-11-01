//! Resolving calls to the functions they reach: by name, narrowed by declared types, imports and owners.

use super::*;

impl Project {
    /// The functions named `short`.
    pub(super) fn named(&self, short: &str) -> Vec<FnId> {
        self.by_short.get(short).cloned().unwrap_or_default()
    }

    pub(super) fn resolve(&mut self) {
        let mut by_short: HashMap<String, Vec<FnId>> = HashMap::new();
        for (id, function) in self.functions.iter().enumerate() {
            by_short
                .entry(function.short().to_string())
                .or_default()
                .push(id);
        }
        self.by_short = by_short;
        let mut types: HashMap<String, Vec<(usize, usize)>> = HashMap::new();
        for (file, source) in self.files.iter().enumerate() {
            for (ix, ty) in source.types.iter().enumerate() {
                types.entry(ty.name.clone()).or_default().push((file, ix));
            }
        }
        self.types = types;
        let edges: Vec<Vec<Edge>> = (0..self.functions.len())
            .map(|id| {
                let function = &self.functions[id];
                function
                    .outline
                    .calls
                    .iter()
                    .enumerate()
                    .filter_map(|(call, site)| {
                        let target = if site.through_parameter {
                            Target::Unknown
                        } else {
                            self.target(id, site)?
                        };
                        if site.reference && !matches!(target, Target::Function(_)) {
                            return None;
                        }
                        Some(Edge {
                            from: id,
                            target,
                            call,
                            callback: site.reference || site.through_parameter,
                        })
                    })
                    .collect()
            })
            .collect();
        let mut callers = vec![Vec::new(); self.functions.len()];
        for edge in edges.iter().flatten() {
            if let Target::Function(to) = edge.target
                && !callers[to].contains(&edge.from)
                && to != edge.from
            {
                callers[to].push(edge.from);
            }
        }
        self.edges = edges;
        self.callers = callers;
    }

    /// What call `site` in function `from` reaches; `None` for calls to
    /// the language's own (built-in) functions, or names matching too
    /// much.
    pub(super) fn target(&self, from: FnId, site: &Call) -> Option<Target> {
        let function = &self.functions[from];
        let file = &self.files[function.file];
        let named = self.named(&site.name);
        let owner = function.outline.owner.as_deref();
        match site.qualifier.as_deref() {
            // On `this`, or nothing: the function's own type and what it
            // extends, the same file, then what the file imports.
            None => {
                if let Some(owner) = owner
                    && let Some(found) = self.method_of(owner, &site.name)
                {
                    return Some(found);
                }
                let here: Vec<FnId> = named
                    .iter()
                    .copied()
                    .filter(|&id| self.functions[id].file == function.file)
                    .collect();
                if let [one] = here.as_slice() {
                    return Some(Target::Function(*one));
                }
                if let Some(found) = self.imported(function.file, &site.name, &site.name) {
                    return Some(found);
                }
                self.by_name(&named)
            }
            // On an expression with no name (`builder().build()`): a guess
            // by name would mostly be wrong, so none.
            Some("") => None,
            Some(qualifier) => {
                let ty = self.type_of(from, qualifier);
                if let Some(ty) = &ty {
                    if let Some(found) = self.method_of(ty, &site.name) {
                        return Some(found);
                    }
                    if self.types.contains_key(ty.as_str()) {
                        // A project type without it: inherited from outside.
                        return None;
                    }
                    if let Some(package) = self.package_of(function.file, ty) {
                        return Some(Target::External(package));
                    }
                }
                // A namespace or module imported by that name.
                if let Some(found) = self.imported(function.file, qualifier, &site.name) {
                    return Some(found);
                }
                // A typed name of the language's own, such as a string, or a
                // Java name the index can't type: not drawn.
                if ty.is_some() || file.lang == Lang::Java {
                    return None;
                }
                match named.len() {
                    1..=MAX_AMBIGUOUS => Some(Target::Ambiguous(named)),
                    _ => None,
                }
            }
        }
    }

    /// One function by name alone: certain if there's one, ambiguous if
    /// there are a few.
    fn by_name(&self, named: &[FnId]) -> Option<Target> {
        match named.len() {
            1 => Some(Target::Function(named[0])),
            2..=MAX_AMBIGUOUS => Some(Target::Ambiguous(named.to_vec())),
            _ => None,
        }
    }

    /// Method `name` of type `ty` or what it extends, in the project.
    fn method_of(&self, ty: &str, name: &str) -> Option<Target> {
        let mut seen = HashSet::new();
        let mut types = vec![ty.to_string()];
        while let Some(ty) = types.pop() {
            if !seen.insert(ty.clone()) {
                continue;
            }
            let found: Vec<FnId> = self
                .named(name)
                .into_iter()
                .filter(|&id| self.functions[id].outline.owner.as_deref() == Some(ty.as_str()))
                .collect();
            match found.as_slice() {
                [] => {}
                [one] => return Some(Target::Function(*one)),
                // Overloads: the first, which the server tells apart.
                [first, ..] => return Some(Target::Function(*first)),
            }
            for &(file, ix) in self.types.get(ty.as_str()).into_iter().flatten() {
                types.extend(self.files[file].types[ix].implements.iter().cloned());
            }
        }
        None
    }

    /// The type `name` is declared with in function `from`: a parameter,
    /// a local, a field of its type; or a type itself, by its capital.
    fn type_of(&self, from: FnId, name: &str) -> Option<String> {
        let function = &self.functions[from];
        if let Some((_, ty)) = function
            .outline
            .locals
            .iter()
            .rev()
            .find(|(n, _)| n == name)
        {
            return Some(ty.clone());
        }
        let file = &self.files[function.file];
        if let Some(owner) = function.outline.owner.as_deref()
            && let Some(ty) = file.types.iter().find(|t| t.name == owner)
            && let Some((_, field)) = ty.fields.iter().find(|(n, _)| n == name)
        {
            return Some(field.clone());
        }
        name.starts_with(char::is_uppercase)
            .then(|| name.to_string())
    }

    /// The package outside the project that file `file` gets type `ty`
    /// from.
    fn package_of(&self, file: usize, ty: &str) -> Option<String> {
        let source = &self.files[file];
        for import in &source.imports {
            if import.names.iter().any(|(local, _)| local == ty) {
                return match source.lang {
                    // A type of the project's own the index can't see
                    // (generated, say) is no package outside.
                    Lang::Java => match self.java_import(&import.path) {
                        JavaImport::Ours { .. } => None,
                        JavaImport::Theirs(package) => Some(package),
                    },
                    _ if import.path.starts_with('.') => None,
                    _ if self.resolve_import(file, &import.path).is_some() => None,
                    _ => Some(npm_package(&import.path)),
                };
            }
        }
        match source.lang {
            Lang::Java => Some("java.lang".into()),
            _ => None,
        }
    }

    /// What `name` (used as `called` is called on it, or called itself)
    /// is, when file `file` imports it: a function in another file of the
    /// project, or a package outside.
    fn imported(&self, file: usize, name: &str, called: &str) -> Option<Target> {
        let source = &self.files[file];
        let import = source
            .imports
            .iter()
            .find(|import| import.names.iter().any(|(local, _)| local == name))?;
        let exported = import
            .names
            .iter()
            .find(|(local, _)| local == name)
            .map(|(_, exported)| exported.as_str())
            .unwrap_or(name);
        match source.lang {
            Lang::TypeScript | Lang::Tsx | Lang::JavaScript => {
                let Some(target) = self.resolve_import(file, &import.path) else {
                    if import.path.starts_with('.') {
                        return None;
                    }
                    return Some(Target::External(npm_package(&import.path)));
                };
                // `fn()` imported as itself, or `ns.fn()` from a namespace.
                let wanted = if name == called { exported } else { called };
                let found: Vec<FnId> = self
                    .named(wanted)
                    .into_iter()
                    .filter(|&id| {
                        self.functions[id].file == target
                            || self.functions[id].outline.owner.as_deref() == Some(name)
                    })
                    .collect();
                self.by_name(&found)
            }
            Lang::Java => match self.java_import(&import.path) {
                // `import static a.b.Type.method`: the type's method.
                JavaImport::Ours {
                    member_of: Some(ty),
                } => self.method_of(&ty, called),
                JavaImport::Ours { member_of: None } => self.method_of(name, called),
                JavaImport::Theirs(package) => Some(Target::External(package)),
            },
            _ => None,
        }
    }

    /// Whether a Java import is of the project's own packages, and of a
    /// type's member (a static import), or which package outside.
    fn java_import(&self, path: &str) -> JavaImport {
        let ours = |package: &str| {
            self.files
                .iter()
                .any(|f| f.package.as_deref() == Some(package))
        };
        let Some((package, _)) = path.rsplit_once('.') else {
            return JavaImport::Theirs(path.to_string());
        };
        if ours(package) {
            return JavaImport::Ours { member_of: None };
        }
        if let Some((outer, ty)) = package.rsplit_once('.')
            && ours(outer)
        {
            return JavaImport::Ours {
                member_of: Some(ty.to_string()),
            };
        }
        JavaImport::Theirs(fold_package(package))
    }

    /// The project file an import from file `from` means: relative, or
    /// through a path alias; `None` for a package.
    pub fn resolve_import(&self, from: usize, path: &str) -> Option<usize> {
        if path.starts_with('.') {
            let dir = self.files[from].path.parent().unwrap_or(Path::new(""));
            return self.file_at(&normalize(&dir.join(path)));
        }
        let file = &self.files[from].path;
        let alias = self
            .aliases
            .iter()
            .filter(|alias| file.starts_with(&alias.dir) && path.starts_with(&alias.prefix))
            .max_by_key(|alias| alias.dir.components().count())?;
        self.file_at(&alias.target.join(&path[alias.prefix.len()..]))
    }

    /// The project file at `joined`, as imports name files: without their
    /// extension, `.js` for `.ts`, or a folder's index.
    fn file_at(&self, joined: &Path) -> Option<usize> {
        let stem = joined.to_string_lossy().into_owned();
        let stem = stem
            .strip_suffix(".js")
            .or_else(|| stem.strip_suffix(".mjs"))
            .unwrap_or(&stem)
            .to_string();
        let tries = [
            format!("{stem}.ts"),
            format!("{stem}.tsx"),
            format!("{stem}.js"),
            format!("{stem}/index.ts"),
            format!("{stem}/index.js"),
            stem.clone(),
        ];
        tries.iter().find_map(|candidate| {
            let candidate = Path::new(candidate);
            self.files.iter().position(|file| file.path == candidate)
        })
    }

    /// Which modules each module imports, from its files' imports.
    pub(super) fn module_imports(&mut self) {
        let mut by_package: HashMap<&str, usize> = HashMap::new();
        for file in &self.files {
            if let Some(package) = file.package.as_deref() {
                by_package.insert(package, file.module);
            }
        }
        let mut imports: Vec<HashMap<usize, usize>> = vec![HashMap::new(); self.modules.len()];
        for (ix, file) in self.files.iter().enumerate() {
            for import in &file.imports {
                let target = match file.lang {
                    Lang::Java => {
                        let package = if import.wildcard {
                            import.path.as_str()
                        } else {
                            import.path.rsplit_once('.').map_or("", |(p, _)| p)
                        };
                        by_package.get(package).copied()
                    }
                    _ => self
                        .resolve_import(ix, &import.path)
                        .map(|file| self.files[file].module),
                };
                if let Some(target) = target.filter(|&target| target != file.module) {
                    *imports[file.module].entry(target).or_default() += 1;
                }
            }
        }
        // Java needs no import within a package, but across: the types a
        // file uses from other packages are imported, so this covers it.
        self.imports = imports
            .into_iter()
            .map(|map| {
                let mut list: Vec<(usize, usize)> = map.into_iter().collect();
                list.sort();
                list
            })
            .collect();
    }
}

enum JavaImport {
    Ours { member_of: Option<String> },
    Theirs(String),
}

/// `org.springframework.data` from `org.springframework.data.jpa.repository`:
/// one node per library, not per package in it.
fn fold_package(package: &str) -> String {
    package.split('.').take(3).collect::<Vec<_>>().join(".")
}

/// `@scope/name` or `name` from an import of `@scope/name/sub`.
fn npm_package(path: &str) -> String {
    let mut parts = path.split('/');
    let first = parts.next().unwrap_or(path);
    if first.starts_with('@') {
        match parts.next() {
            Some(second) => format!("{first}/{second}"),
            None => first.to_string(),
        }
    } else {
        first.to_string()
    }
}
