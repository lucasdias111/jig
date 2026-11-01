//! Building the index from what was read of each file, and labelling its
//! modules.

use super::*;

impl Project {
    /// File `file` as it read: its outline, functions and all.
    pub(crate) fn outline_of(&self, file: usize) -> Outline {
        let source = &self.files[file];
        Outline {
            purpose: source.purpose.clone(),
            package: source.package.clone(),
            imports: source.imports.clone(),
            types: source.types.clone(),
            functions: self.modules[source.module]
                .functions
                .iter()
                .map(|&id| &self.functions[id])
                .filter(|function| function.file == file)
                .map(|function| function.outline.clone())
                .collect(),
            variables: source.variables.clone(),
        }
    }

    /// The project made of these files, their stamps, and what they read
    /// as.
    pub(crate) fn from_stamped(
        root: &Path,
        files: Vec<(PathBuf, Option<Stamp>, Outline)>,
        aliases: Vec<Alias>,
    ) -> Project {
        let mut project = Project {
            root: root.to_path_buf(),
            aliases,
            ..Project::default()
        };
        let mut dirs: HashMap<PathBuf, usize> = HashMap::new();
        for (path, stamp, outline) in files {
            let Some(lang) = lang_of(&path) else {
                continue;
            };
            let dir = path.parent().map(Path::to_path_buf).unwrap_or_default();
            let module = *dirs.entry(dir.clone()).or_insert_with(|| {
                project.modules.push(Module {
                    dir: dir.clone(),
                    label: String::new(),
                    files: Vec::new(),
                    functions: Vec::new(),
                });
                project.modules.len() - 1
            });
            let file = project.files.len();
            project.modules[module].files.push(file);
            for function in outline.functions {
                project.modules[module]
                    .functions
                    .push(project.functions.len());
                project.functions.push(Func {
                    file,
                    outline: function,
                });
            }
            project.files.push(SourceFile {
                path,
                lang,
                purpose: outline.purpose,
                stamp,
                module,
                package: outline.package,
                imports: outline.imports,
                types: outline.types,
                variables: outline.variables,
            });
        }
        project.label_modules();
        project.resolve();
        project.module_imports();
        project
    }

    /// Modules are shown by package past the part every package shares, or
    /// by folder.
    fn label_modules(&mut self) {
        let packages: Vec<&str> = self
            .files
            .iter()
            .filter_map(|file| file.package.as_deref())
            .collect();
        let shared = packages.iter().skip(1).fold(
            packages
                .first()
                .map(|p| p.split('.').collect::<Vec<_>>())
                .unwrap_or_default(),
            |shared, package| {
                shared
                    .into_iter()
                    .zip(package.split('.'))
                    .take_while(|(a, b)| a == b)
                    .map(|(a, _)| a)
                    .collect()
            },
        );
        // Keep one part of what's shared, so the top package has a name.
        let keep = shared.len().saturating_sub(1);
        for module in &mut self.modules {
            let package = module
                .files
                .first()
                .and_then(|&file| self.files[file].package.as_deref());
            module.label = match package {
                Some(package) => package.split('.').skip(keep).collect::<Vec<_>>().join("."),
                None if module.dir.as_os_str().is_empty() => "(top)".into(),
                None => module.dir.to_string_lossy().replace('\\', "/"),
            };
        }
    }
}
