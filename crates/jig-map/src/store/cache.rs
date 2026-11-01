//! The index, a cache of the project on disk: its files found and read
//! (again only where they changed), for the domain to build the index
//! from (`domain::index`). Reading happens on a background thread the
//! view starts.

mod walk;

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::domain::files::Sources;
use crate::domain::index::{Project, Stamp, lang_of};
use crate::domain::outline::{self, Outline};

/// The project's folder, read for what the index doesn't keep.
pub struct Folder<'a>(pub &'a Path);

impl Folder<'_> {
    /// The text of file `place`, relative to the project.
    pub fn text(&self, place: &Path) -> Option<String> {
        std::fs::read_to_string(self.0.join(place)).ok()
    }
}

impl Sources for Folder<'_> {
    fn read(&self, place: &Path) -> Option<Vec<u8>> {
        std::fs::read(self.0.join(place)).ok()
    }
}

impl Project {
    /// Read the project at `root`.
    #[cfg(any(test, feature = "testing"))]
    pub fn read(root: &Path) -> Project {
        Project::read_again(root, None)
    }

    /// Read the project at `root` again, keeping what `before` read of
    /// each file unchanged since.
    pub fn read_again(root: &Path, before: Option<&Project>) -> Project {
        let found = walk::source_files(root);
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
        let aliases = walk::aliases(root, &found.configs);
        let mut project = Project::from_stamped(root, files, aliases);
        project.others = found.others;
        project.other_lengths = found.other_lengths;
        project.packages = found.packages;
        project.readmes = found.readmes;
        project.skipped = found.skipped;
        project
    }
}

/// The project `file` is in, when no folder is open: the nearest folder
/// above it with `.git`, else with a manifest (`Cargo.toml`,
/// `package.json`…), else its own folder.
pub fn root_of(file: &Path) -> PathBuf {
    let dir = file.parent().unwrap_or(Path::new("."));
    let with = |marker: &dyn Fn(&Path) -> bool| dir.ancestors().find(|dir| marker(dir));
    with(&|dir| dir.join(".git").exists())
        .or_else(|| with(&|dir| walk::MANIFESTS.iter().any(|name| dir.join(name).is_file())))
        .unwrap_or(dir)
        .to_path_buf()
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
