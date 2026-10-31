//! Reading the project: outlining its files, again only where they changed, and labelling its modules.

use super::*;

impl Project {
    /// Read the project at `root`.
    #[cfg(any(test, feature = "testing"))]
    pub fn read(root: &Path) -> Project {
        Project::read_again(root, None)
    }

    /// Read the project at `root` again, keeping what `before` read of
    /// each file unchanged since.
    pub fn read_again(root: &Path, before: Option<&Project>) -> Project {
        let found = source_files(root);
        let kept: HashMap<&Path, usize> = before
            .filter(|before| before.root == root)
            .map(|before| {
                before
                    .files
                    .iter()
                    .enumerate()
                    .map(|(ix, file)| (file.path.as_path(), ix))
                    .collect()
            })
            .unwrap_or_default();
        let unchanged = |(path, stamp): &(PathBuf, Option<Stamp>)| {
            let ix = kept.get(path.as_path())?;
            let before = before?;
            (stamp.is_some() && before.files[*ix].stamp == *stamp).then_some(*ix)
        };
        let changed: Vec<PathBuf> = found
            .sources
            .iter()
            .filter(|source| unchanged(source).is_none())
            .map(|(path, _)| path.clone())
            .collect();
        let mut read = read_all(root, &changed).into_iter();
        let files = found
            .sources
            .iter()
            .map(|source| {
                let outline = match (unchanged(source), before) {
                    (Some(ix), Some(before)) => before.outline_of(ix),
                    _ => read.next().unwrap_or_default(),
                };
                (source.0.clone(), source.1, outline)
            })
            .collect();
        let aliases = aliases(root, &found.configs);
        let mut project = Project::from_stamped(root, files, aliases);
        project.others = found.others;
        project.packages = found.packages;
        project.readmes = found.readmes;
        project.skipped = found.skipped;
        project
    }

    /// File `file` as it read: its outline, functions and all.
    fn outline_of(&self, file: usize) -> Outline {
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
    fn from_stamped(
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

/// Read every file, on as many threads as there are cores.
fn read_all(root: &Path, paths: &[PathBuf]) -> Vec<Outline> {
    let threads = std::thread::available_parallelism()
        .map_or(4, |n| n.get())
        .min(12);
    let chunk = paths.len().div_ceil(threads).max(1);
    std::thread::scope(|scope| {
        let handles: Vec<_> = paths
            .chunks(chunk)
            .map(|chunk| {
                scope.spawn(move || {
                    chunk
                        .iter()
                        .map(|path| {
                            let lang = lang_of(path).expect("listed by language");
                            std::fs::read_to_string(root.join(path))
                                .map(|source| outline::outline(&source, lang))
                                .unwrap_or_default()
                        })
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        handles
            .into_iter()
            .flat_map(|handle| handle.join().unwrap_or_default())
            .collect()
    })
}
