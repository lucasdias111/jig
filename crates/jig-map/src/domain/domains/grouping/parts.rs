//! Finding the groups: opening folders that hold most of the code, and placing loose files.

use super::*;

/// Down a chain of folders holding one folder and nothing else.
pub(super) fn descend(mut part: Part, files: &[PathBuf]) -> Part {
    loop {
        let next: Vec<Option<std::path::Component>> = part
            .files
            .iter()
            .map(|&f| {
                let rest = files[f].strip_prefix(&part.dir).ok()?;
                let mut parts = rest.components();
                let first = parts.next()?;
                // A file right here: no folder to go down into.
                parts.next()?;
                Some(first)
            })
            .collect();
        match next.first() {
            Some(Some(first)) if next.iter().all(|n| n.as_ref() == Some(first)) => {
                part.dir = part.dir.join(first);
            }
            _ => return part,
        }
    }
}

/// A folder's folders, each a part, and the files loose in it, one more.
pub(super) fn open_part(part: Part, files: &[PathBuf]) -> Vec<Part> {
    let mut folders: Vec<(PathBuf, Vec<usize>)> = Vec::new();
    let mut loose = Vec::new();
    for &f in &part.files {
        let rest = files[f].strip_prefix(&part.dir).unwrap_or(&files[f]);
        let mut components = rest.components();
        let first = components.next();
        match (first, components.next()) {
            (Some(first), Some(_)) => {
                let dir = part.dir.join(first);
                match folders.iter_mut().find(|(d, _)| *d == dir) {
                    Some((_, list)) => list.push(f),
                    None => folders.push((dir, vec![f])),
                }
            }
            _ => loose.push(f),
        }
    }
    folders.sort_by(|a, b| a.0.cmp(&b.0));
    let mut parts: Vec<Part> = folders
        .into_iter()
        .map(|(dir, files_in)| {
            descend(
                Part {
                    dir,
                    loose: false,
                    files: files_in,
                    joined: Vec::new(),
                    merged: Vec::new(),
                },
                files,
            )
        })
        .collect();
    if !loose.is_empty() {
        parts.push(Part {
            dir: part.dir,
            loose: true,
            files: loose,
            joined: Vec::new(),
            merged: Vec::new(),
        });
    }
    parts
}

/// The certain calls from functions in one file to those in another,
/// counted, by index into `files`.
pub(super) fn file_calls(project: &Project, files: &[PathBuf]) -> HashMap<(usize, usize), usize> {
    let at: HashMap<&Path, usize> = files
        .iter()
        .enumerate()
        .map(|(ix, path)| (path.as_path(), ix))
        .collect();
    let of_read: Vec<Option<usize>> = project
        .files
        .iter()
        .map(|file| at.get(file.path.as_path()).copied())
        .collect();
    let mut calls = HashMap::new();
    for (id, edges) in project.edges.iter().enumerate() {
        let Some(from) = of_read[project.functions[id].file] else {
            continue;
        };
        for edge in edges {
            if let Target::Function(to) = edge.target
                && let Some(into) = of_read[project.functions[to].file]
                && into != from
            {
                *calls.entry((from, into)).or_default() += 1;
            }
        }
    }
    calls
}

/// Loose files beside folders: each joins the folder of its name, else
/// the group it calls and is called by most.
pub(super) fn join_loose_files(
    parts: &mut [Part],
    files: &[PathBuf],
    calls: &HashMap<(usize, usize), usize>,
) {
    let of_file = |parts: &[Part]| -> HashMap<usize, usize> {
        parts
            .iter()
            .enumerate()
            .flat_map(|(ix, part)| part.files.iter().map(move |&f| (f, ix)))
            .collect()
    };
    let homes = of_file(parts);
    let mut moves: Vec<(usize, usize, usize)> = Vec::new();
    for (ix, part) in parts.iter().enumerate().filter(|(_, part)| part.loose) {
        for &f in &part.files {
            let stem = files[f].file_stem().map(|s| part.dir.join(s));
            let named = parts
                .iter()
                .position(|other| !other.loose && Some(&other.dir) == stem.as_ref());
            let called = || {
                let mut with: HashMap<usize, usize> = HashMap::new();
                for (&(a, b), &count) in calls {
                    let other = if a == f {
                        b
                    } else if b == f {
                        a
                    } else {
                        continue;
                    };
                    if let Some(&group) = homes.get(&other)
                        && group != ix
                    {
                        *with.entry(group).or_default() += count;
                    }
                }
                with.into_iter()
                    .filter(|&(_, count)| count >= CALLS_TO_JOIN)
                    .max_by_key(|&(group, count)| (count, std::cmp::Reverse(group)))
                    .map(|(group, _)| group)
            };
            if let Some(to) = named.or_else(called) {
                moves.push((f, ix, to));
            }
        }
    }
    for (f, from, to) in moves {
        parts[from].files.retain(|&x| x != f);
        parts[to].joined.push(f);
    }
}

/// While there are too many groups, the smallest joins the one it makes
/// the most calls with, else its nearest folder.
pub(super) fn merge_smallest(
    parts: &mut Vec<Part>,
    files: &[PathBuf],
    calls: &HashMap<(usize, usize), usize>,
    weigh: &impl Fn(&Part) -> u64,
) {
    while parts.len() > MAX_GROUPS {
        let smallest = (0..parts.len())
            .min_by_key(|&ix| weigh(&parts[ix]))
            .expect("more than one part");
        let held: std::collections::HashSet<usize> = parts[smallest]
            .files
            .iter()
            .chain(&parts[smallest].joined)
            .copied()
            .collect();
        let mut with: HashMap<usize, usize> = HashMap::new();
        for (ix, part) in parts.iter().enumerate().filter(|(ix, _)| *ix != smallest) {
            let theirs: std::collections::HashSet<usize> =
                part.files.iter().chain(&part.joined).copied().collect();
            let count: usize = calls
                .iter()
                .filter(|((a, b), _)| {
                    (held.contains(a) && theirs.contains(b))
                        || (held.contains(b) && theirs.contains(a))
                })
                .map(|(_, count)| count)
                .sum();
            with.insert(ix, count);
        }
        let shared = |ix: usize| {
            parts[smallest]
                .dir
                .components()
                .zip(parts[ix].dir.components())
                .take_while(|(a, b)| a == b)
                .count()
        };
        let into = with
            .iter()
            .max_by_key(|&(&ix, &count)| (count, shared(ix), std::cmp::Reverse(ix)))
            .map(|(&ix, _)| ix)
            .expect("another part");
        let part = parts.remove(smallest);
        let into = if into > smallest { into - 1 } else { into };
        let members = members(&part, files);
        let target = &mut parts[into];
        target.merged.extend(members);
        target.joined.extend(part.files.iter().chain(&part.joined));
    }
}

/// A part's members as domains name them: its folder, or its loose files,
/// with the files and parts that joined it.
pub(super) fn members(part: &Part, files: &[PathBuf]) -> Vec<String> {
    let mut members: Vec<String> = if part.loose || part.dir.as_os_str().is_empty() {
        part.files.iter().map(|&f| place_of(&files[f])).collect()
    } else {
        vec![format!("{}/", place_of(&part.dir))]
    };
    if part.merged.is_empty() {
        members.extend(part.joined.iter().map(|&f| place_of(&files[f])));
    } else {
        // Whole parts merged in are named as they were; files that joined
        // this one by name or calls, one by one.
        members.extend(part.merged.iter().cloned());
        let named: Vec<usize> = part
            .joined
            .iter()
            .copied()
            .filter(|&f| {
                let place = place_of(&files[f]);
                !part
                    .merged
                    .iter()
                    .any(|m| *m == place || (m.ends_with('/') && place.starts_with(m.as_str())))
            })
            .collect();
        members.extend(named.into_iter().map(|f| place_of(&files[f])));
    }
    members
}
