//! Resolving the places a drawing names, or suggesting names like them.

use super::*;

/// `place` in `project`, or names like it to suggest.
pub fn resolve(
    project: &Project,
    domains: Option<&Domains>,
    saved: &dyn SavedSketches,
    place: &str,
) -> Result<Place, Vec<String>> {
    let named = normal(project, place);
    if named.starts_with(super::super::sketch::PREFIX) {
        return resolve_sketch(saved, &named);
    }
    if let Some(name) = named.strip_prefix("domain:") {
        let name = name.trim();
        return match domains
            .and_then(|d| d.domains.iter().find(|d| d.name.eq_ignore_ascii_case(name)))
        {
            Some(domain) => Ok(Place::Domain(domain.name.clone())),
            None => Err(domains
                .map(|d| {
                    d.domains
                        .iter()
                        .map(|d| format!("domain:{}", d.name))
                        .collect()
                })
                .unwrap_or_default()),
        };
    }
    let (path, inner) = match named.split_once('#') {
        Some((path, inner)) => (path.to_string(), Some(inner.trim_end_matches("()"))),
        None => (named.clone(), None),
    };
    let read = project.files.iter().position(|f| slashed(&f.path) == path);
    let real = read.is_some()
        || project.others.iter().any(|f| slashed(f) == path)
        || (!path.ends_with('/') && project.root.join(&path).is_file());
    if !real {
        let dir = format!("{}/", path.trim_end_matches('/'));
        let folder = !path.is_empty()
            && (project
                .files
                .iter()
                .any(|f| slashed(&f.path).starts_with(&dir))
                || project.others.iter().any(|f| slashed(f).starts_with(&dir))
                || project.root.join(&dir).is_dir());
        if folder && inner.is_none() {
            return Ok(Place::Folder(dir));
        }
        return Err(suggest(project, &path, inner));
    }
    let file = |owner: Option<String>, function: Option<String>| Place::File {
        path: path.clone(),
        owner,
        function,
    };
    let (Some(inner), Some(ix)) = (inner.filter(|i| !i.is_empty()), read) else {
        // A file as a whole, or one the index can't read, by any name in
        // it: its box is the file's.
        return Ok(file(None, None));
    };
    let functions: Vec<&super::super::index::Func> =
        project.functions.iter().filter(|f| f.file == ix).collect();
    // `#L12`: the function on that line.
    if let Some(line) = inner
        .strip_prefix('L')
        .and_then(|line| line.parse::<u32>().ok())
        && let Some(id) = project.function_on_line(&project.files[ix].path, line.saturating_sub(1))
    {
        let function = &project.functions[id];
        return Ok(file(function.outline.owner.clone(), Some(function.title())));
    }
    let found = functions
        .iter()
        .find(|f| f.title() == inner)
        .or_else(|| functions.iter().find(|f| f.short() == inner))
        .or_else(|| {
            // `Type.method` where the index has it under another owner.
            let short = inner.rsplit(['.', ':']).next()?;
            functions.iter().find(|f| f.short() == short)
        });
    if let Some(function) = found {
        return Ok(file(function.outline.owner.clone(), Some(function.title())));
    }
    let types = &project.files[ix].types;
    if functions
        .iter()
        .any(|f| f.outline.owner.as_deref() == Some(inner))
        || types.iter().any(|ty| ty.name == inner)
    {
        return Ok(file(Some(inner.to_string()), None));
    }
    // The file's functions, those most like the name first.
    let wanted = inner
        .rsplit(['.', ':'])
        .next()
        .unwrap_or(inner)
        .to_lowercase();
    let mut like: Vec<&&super::super::index::Func> = functions.iter().collect();
    like.sort_by_key(|f| std::cmp::Reverse(likeness(&f.short().to_lowercase(), &wanted)));
    Err(like
        .into_iter()
        .take(8)
        .map(|f| format!("{path}#{}", f.title()))
        .collect())
}

/// A box of a sketch saved in the project, by its id or label;
/// else the sketches' boxes, to suggest.
pub(super) fn resolve_sketch(saved: &dyn SavedSketches, name: &str) -> Result<Place, Vec<String>> {
    let sketches = saved.sketches();
    let all = || {
        sketches
            .iter()
            .flat_map(|s| {
                s.boxes
                    .iter()
                    .map(|b| super::super::sketch::place_name(&s.name, &b.id))
            })
            .take(12)
            .collect()
    };
    let Some((wanted, shape)) = super::super::sketch::parse_place(name) else {
        return Err(all());
    };
    let same =
        |a: &str, b: &str| super::super::sketch::slug(a, "") == super::super::sketch::slug(b, "");
    let found = sketches
        .iter()
        .find(|s| s.name == wanted || same(&s.title, &wanted))
        .and_then(|s| {
            let found = s
                .boxes
                .iter()
                .find(|b| b.id == shape || same(&b.label, &shape))?;
            Some(Place::Sketch {
                sketch: s.name.clone(),
                shape: found.id.clone(),
            })
        });
    found.ok_or_else(all)
}

/// How alike two names are: the start they share, more when one holds the
/// other.
pub(super) fn likeness(a: &str, b: &str) -> usize {
    let shared = a.chars().zip(b.chars()).take_while(|(x, y)| x == y).count();
    let held = if !b.is_empty() && (a.contains(b) || b.contains(a)) {
        b.len().min(a.len())
    } else {
        0
    };
    shared + held
}

/// A name as the index keeps paths: relative to the project, with `/`,
/// without `./` or a line.
pub(super) fn normal(project: &Project, place: &str) -> String {
    let mut place = place.trim().trim_matches('`').replace('\\', "/");
    let root = slashed(&project.root);
    if let Some(rest) = place.strip_prefix(&format!("{}/", root.trim_end_matches('/'))) {
        place = rest.to_string();
    }
    while let Some(rest) = place.strip_prefix("./") {
        place = rest.to_string();
    }
    // `path:12` is the function on that line.
    if !place.contains('#')
        && let Some((path, line)) = place.rsplit_once(':')
        && line.chars().all(|c| c.is_ascii_digit())
        && !line.is_empty()
    {
        place = format!("{path}#L{line}");
    }
    place
}

/// Up to six places named like what was asked for.
pub(super) fn suggest(project: &Project, path: &str, inner: Option<&str>) -> Vec<String> {
    let last = |text: &str| {
        text.rsplit(['.', '/', '#'])
            .find(|part| !part.is_empty())
            .unwrap_or_default()
            .to_lowercase()
    };
    let mut like: Vec<String> = Vec::new();
    // Files named the same, wherever they are.
    let file_name = path.rsplit('/').next().unwrap_or(path).to_lowercase();
    if !file_name.is_empty() {
        for file in project
            .files
            .iter()
            .map(|f| slashed(&f.path))
            .chain(project.others.iter().map(|f| slashed(f)))
        {
            if file.to_lowercase().ends_with(&format!("/{file_name}")) || file == file_name {
                like.push(match inner {
                    Some(inner) => format!("{file}#{inner}"),
                    None => file,
                });
            }
        }
    }
    let wanted = last(inner.unwrap_or(path));
    if !wanted.is_empty() {
        for function in &project.functions {
            if like.len() >= 6 {
                break;
            }
            if function.short().to_lowercase() == wanted {
                like.push(format!(
                    "{}#{}",
                    slashed(&project.files[function.file].path),
                    function.title()
                ));
            }
        }
    }
    like.truncate(6);
    like
}

/// Each place `drawing` names resolved, or why not: every name that isn't
/// found, with names like it.
pub fn check(
    project: &Project,
    domains: Option<&Domains>,
    saved: &dyn SavedSketches,
    drawing: &Drawing,
) -> Result<(), String> {
    let mut missing = Vec::new();
    for place in drawing.places() {
        if let Err(like) = resolve(project, domains, saved, place) {
            missing.push(if like.is_empty() {
                format!("{place} (nothing like it)")
            } else {
                format!("{place} (did you mean {})", like.join(", "))
            });
        }
    }
    if missing.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "Nothing was drawn: these don't name a place in the project: {}. Name each as path#Type.method, path#function, path, folder/, domain:Name or sketch:<sketch>/<box>, paths relative to the project, and draw again.",
            missing.join("; ")
        ))
    }
}
