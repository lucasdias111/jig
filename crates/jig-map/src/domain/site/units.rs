//! What Site is made of: components, and which of their files show.

use super::*;

/// The files and types each region holds, and which homes are used.
pub(super) fn units(project: &Project, domains: &Domains, show: Show) -> (Vec<Unit>, Vec<Home>) {
    let read: HashMap<&Path, usize> = project
        .files
        .iter()
        .enumerate()
        .map(|(ix, file)| (file.path.as_path(), ix))
        .collect();
    let mut functions: HashMap<(usize, Option<&str>), usize> = HashMap::new();
    for function in &project.functions {
        *functions.entry((function.file, None)).or_default() += 1;
        if let Some(owner) = &function.outline.owner {
            *functions
                .entry((function.file, Some(owner.as_str())))
                .or_default() += 1;
        }
    }
    let mut units = Vec::new();
    let mut homes = Vec::new();
    for path in all_files(project) {
        let place = place_of(&path);
        let home = domains.home_of(&place);
        if !homes.contains(&home) {
            homes.push(home);
        }
        if !show.tests && is_test(&path) {
            continue;
        }
        let file = read.get(path.as_path()).copied();
        let mut count = file
            .and_then(|ix| functions.get(&(ix, None)))
            .copied()
            .unwrap_or(0);
        let mut types = Vec::new();
        if let Some(ix) = file {
            for ty in &project.files[ix].types {
                let n = functions
                    .get(&(ix, Some(ty.name.as_str())))
                    .copied()
                    .unwrap_or(0);
                let own = domains.home_of(&format!("{place}#{}", ty.name));
                if own == home {
                    types.push((ty.name.clone(), ty.kind, ty.purpose.clone(), n));
                    continue;
                }
                if !homes.contains(&own) {
                    homes.push(own);
                }
                count = count.saturating_sub(n);
                units.push(Unit {
                    path: path.clone(),
                    place: place.clone(),
                    home: own,
                    file,
                    own_type: Some(ty.name.clone()),
                    purpose: ty.purpose.clone(),
                    functions: n,
                    types: Vec::new(),
                });
            }
        }
        // A file holding one type is that type's box.
        let purpose = file.and_then(|ix| {
            project.files[ix]
                .purpose
                .clone()
                .or_else(|| match types.as_slice() {
                    [(_, _, purpose, _)] => purpose.clone(),
                    _ => None,
                })
        });
        units.push(Unit {
            path,
            place,
            home,
            file,
            own_type: None,
            purpose,
            functions: count,
            types,
        });
    }
    (units, homes)
}

/// The component holding `path`: the deepest package (a folder with a
/// manifest) it's in, else its top folder, else the project.
pub(super) fn component_of(project: &Project, path: &Path) -> PathBuf {
    let dir = path.parent().unwrap_or(Path::new(""));
    project
        .packages
        .iter()
        .filter(|package| !package.as_os_str().is_empty() && dir.starts_with(package))
        .max_by_key(|package| package.components().count())
        .cloned()
        .unwrap_or_else(|| {
            dir.components()
                .next()
                .map(|c| PathBuf::from(c.as_os_str()))
                .unwrap_or_default()
        })
}

/// What folder `dir` shows: its folders, with those that only lead
/// somewhere (a chain of single folders, `src`, `main`) folded away, and
/// the units right in it.
pub(super) fn shown<'a>(dir: &'a Dir, at: &Path) -> (Vec<(String, PathBuf, &'a Dir)>, &'a [usize]) {
    fn lift<'a>(
        name: &str,
        path: PathBuf,
        dir: &'a Dir,
        out: &mut Vec<(String, PathBuf, &'a Dir)>,
    ) {
        if dir.units.is_empty() && (dir.dirs.len() == 1 || BOILERPLATE.contains(&name)) {
            for (inner, sub) in &dir.dirs {
                lift(inner, path.join(inner), sub, out);
            }
        } else {
            out.push((name.to_string(), path, dir));
        }
    }
    let mut out = Vec::new();
    for (name, sub) in &dir.dirs {
        lift(name, at.join(name), sub, &mut out);
    }
    (out, &dir.units)
}

/// The deepest folder a component leads straight down to.
pub(super) fn deepest_place(dir: &Path, tree: &Dir) -> String {
    let mut path = dir.to_path_buf();
    let mut at = tree;
    while at.units.is_empty() && at.dirs.len() == 1 {
        let (name, inner) = at.dirs.iter().next().expect("one");
        path = path.join(name);
        at = inner;
    }
    slashed(&path)
}
