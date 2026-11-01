//! Matching a sketch's boxes and arrows to the code, as the project is read again.

use super::*;

/// What a box's label matches in the code.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Match {
    /// Nothing yet: still an idea.
    None,
    /// This place, for sure.
    Real(String),
    /// Several things it could be: left sketched.
    Unsure(usize),
}

impl Match {
    pub fn place(&self) -> Option<&str> {
        match self {
            Match::Real(place) => Some(place),
            _ => None,
        }
    }
}

/// A label as names are compared: letters and digits, lower case, so
/// `Rate limiter`, `RateLimiter` and `rate_limiter.rs` are the same.
pub(super) fn normal(name: &str) -> String {
    name.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

/// What box `label` is in `project`, placed near `near` (the folder or
/// file of the real box it was drawn by): a type, a function or a file of
/// that name. Several in one file are one thing (a type, its file and its
/// constructor); several in different files are narrowed to those near
/// the box, and if that doesn't leave one, it's unsure.
pub fn match_box(project: &Project, label: &str, near: Option<&str>) -> Match {
    let wanted = normal(label.split('(').next().unwrap_or(label));
    if wanted.chars().count() < 3 {
        return Match::None;
    }
    // Candidates by file: the best name in each (a type over a function
    // over the file itself).
    let mut found: Vec<(String, u8, String)> = Vec::new();
    let mut add =
        |file: String, rank: u8, place: String| match found.iter_mut().find(|(f, _, _)| *f == file)
        {
            Some(entry) if entry.1 < rank => *entry = (file, rank, place),
            Some(_) => {}
            None => found.push((file, rank, place)),
        };
    for file in &project.files {
        let path = slashed(&file.path);
        for ty in &file.types {
            if normal(&ty.name) == wanted {
                add(path.clone(), 3, format!("{path}#{}", ty.name));
            }
        }
        if normal(stem(&path)) == wanted {
            add(path.clone(), 1, path.clone());
        }
    }
    for function in &project.functions {
        if normal(function.short()) == wanted || normal(&function.title()) == wanted {
            let path = slashed(&project.files[function.file].path);
            add(path.clone(), 2, format!("{path}#{}", function.title()));
        }
    }
    for other in &project.others {
        let path = slashed(other);
        if normal(stem(&path)) == wanted {
            add(path.clone(), 1, path);
        }
    }
    match found.len() {
        0 => Match::None,
        1 => Match::Real(found.remove(0).2),
        n => {
            let Some(near) = near.map(folder_of).filter(|dir| !dir.is_empty()) else {
                return Match::Unsure(n);
            };
            // In the box's own folder first, then anywhere under it.
            let here: Vec<&(String, u8, String)> = found
                .iter()
                .filter(|(file, _, _)| folder_of(file) == near)
                .collect();
            let under: Vec<&(String, u8, String)> = found
                .iter()
                .filter(|(file, _, _)| file.starts_with(&near))
                .collect();
            match (here.as_slice(), under.as_slice()) {
                ([one], _) | ([], [one]) => Match::Real(one.2.clone()),
                _ => Match::Unsure(n),
            }
        }
    }
}

/// The folder a place is in (`a/b/` for `a/b/c.rs#T`, itself for a folder).
pub(super) fn folder_of(place: &str) -> String {
    let path = place.split('#').next().unwrap_or(place);
    if path.ends_with('/') {
        return path.to_string();
    }
    match path.rfind('/') {
        Some(at) => path[..=at].to_string(),
        None => String::new(),
    }
}

/// The functions place `place` holds: itself, a type's, a file's or a
/// folder's.
pub fn functions_in(project: &Project, place: &Place) -> Vec<FnId> {
    if let Some(id) = place.function_in(project) {
        return vec![id];
    }
    let in_file = |file: usize, owner: Option<&str>| -> Vec<FnId> {
        project
            .functions
            .iter()
            .enumerate()
            .filter(|(_, f)| {
                f.file == file
                    && owner.is_none_or(|owner| f.outline.owner.as_deref() == Some(owner))
            })
            .map(|(id, _)| id)
            .collect()
    };
    match place {
        Place::File { path, owner, .. } => project
            .files
            .iter()
            .position(|f| slashed(&f.path) == *path)
            .map(|file| in_file(file, owner.as_deref()))
            .unwrap_or_default(),
        Place::Folder(dir) => project
            .files
            .iter()
            .enumerate()
            .filter(|(_, f)| slashed(&f.path).starts_with(dir.as_str()))
            .flat_map(|(file, _)| in_file(file, None))
            .collect(),
        Place::Domain(_) | Place::Sketch { .. } => Vec::new(),
    }
}

/// Whether anything in `from` surely calls anything in `to`.
pub fn calls_between(project: &Project, from: &[FnId], to: &[FnId]) -> bool {
    let to: HashSet<FnId> = to.iter().copied().collect();
    from.iter().any(|&id| {
        project.edges.get(id).is_some_and(|edges| {
            edges.iter().any(|edge| match &edge.target {
                Target::Function(target) => to.contains(target),
                // A guess by name alone isn't a call to snap to.
                _ => false,
            })
        })
    })
}
