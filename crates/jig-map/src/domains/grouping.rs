//! Jig's own grouping of a project, before the agent has named its
//! domains: drafts from the index alone. Folders are the start: the
//! project's top folders, opened while there are too few of them, and any
//! folder holding most of the code split into its folders. Files lying
//! loose beside those folders join the folder sharing their name
//! (`git.rs` and `git/`), else the group they make the most calls with.
//! Each group is labelled by the words its functions' names use most.
//!
//! It's also what Index Domains hands the agent as a hint: every source
//! file, by group, and the calls between groups.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use super::super::index::{Project, Target};
use super::{Domain, Domains, Flow, place_of};

/// Fewer groups than this, and the biggest is opened.
const MIN_GROUPS: usize = 3;
/// More than this, and the smallest join their neighbours.
const MAX_GROUPS: usize = 12;
/// A group holding more than this share of the code is split.
const MOST: f32 = 0.5;
/// How many calls make a loose file belong with a group.
const CALLS_TO_JOIN: usize = 2;

#[derive(Clone, Debug, PartialEq)]
pub struct Group {
    /// Its commonest words: `provider · file · model`.
    pub words: String,
    /// Where it is: `crates/jig-ai`, or `crates/jig-app/src` and more.
    pub folder: String,
    /// As domains name members: `dir/`, or files.
    pub members: Vec<String>,
    /// Every file it holds, relative.
    pub files: Vec<PathBuf>,
    pub functions: usize,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Grouping {
    pub groups: Vec<Group>,
    /// The calls from one group to another, counted.
    pub calls: Vec<((usize, usize), usize)>,
}

/// A group while it's worked out: a folder with everything under it, or
/// just the files lying loose in it; and files that joined it.
#[derive(Clone, Debug)]
struct Part {
    dir: PathBuf,
    loose: bool,
    files: Vec<usize>,
    /// Files placed here from elsewhere, named one by one.
    joined: Vec<usize>,
    /// Parts merged into this one, as their members.
    merged: Vec<String>,
}

impl Grouping {
    pub fn of(project: &Project) -> Grouping {
        let files = super::all_files(project);
        let stamps: HashMap<&Path, u64> = project
            .files
            .iter()
            .filter_map(|file| Some((file.path.as_path(), file.stamp?.1)))
            .collect();
        let weights: Vec<u64> = files
            .iter()
            .map(|path| weight(project, &stamps, path))
            .collect();
        let total: u64 = weights.iter().sum::<u64>().max(1);
        let weigh = |part: &Part| -> u64 {
            part.files
                .iter()
                .chain(&part.joined)
                .map(|&f| weights[f])
                .sum()
        };
        let all = Part {
            dir: PathBuf::new(),
            loose: false,
            files: (0..files.len()).collect(),
            joined: Vec::new(),
            merged: Vec::new(),
        };
        let mut parts = vec![descend(all, &files)];
        // Too few: open the biggest folder. One holding most of the code:
        // split it.
        for _ in 0..12 {
            let open = parts
                .iter()
                .enumerate()
                .filter(|(_, part)| !part.loose && opens(part, &files))
                .max_by_key(|(_, part)| weigh(part))
                .map(|(ix, part)| (ix, weigh(part)));
            let Some((ix, heaviest)) = open else {
                break;
            };
            let too_few = parts.len() < MIN_GROUPS;
            let too_big = heaviest as f32 > total as f32 * MOST;
            if !too_few && !too_big {
                break;
            }
            let part = parts.remove(ix);
            let inner = open_part(part, &files);
            parts.splice(ix..ix, inner);
        }
        let file_calls = file_calls(project, &files);
        join_loose_files(&mut parts, &files, &file_calls);
        parts.retain(|part| !part.files.is_empty() || !part.joined.is_empty());
        merge_smallest(&mut parts, &files, &file_calls, &weigh);

        let of_file: HashMap<usize, usize> = parts
            .iter()
            .enumerate()
            .flat_map(|(ix, part)| part.files.iter().chain(&part.joined).map(move |&f| (f, ix)))
            .collect();
        let mut calls: HashMap<(usize, usize), usize> = HashMap::new();
        for (&(from, to), &count) in &file_calls {
            if let (Some(&a), Some(&b)) = (of_file.get(&from), of_file.get(&to))
                && a != b
            {
                *calls.entry((a, b)).or_default() += count;
            }
        }
        let mut calls: Vec<((usize, usize), usize)> = calls.into_iter().collect();
        calls.sort();
        let by_path: HashMap<&Path, usize> = project
            .files
            .iter()
            .enumerate()
            .map(|(ix, file)| (file.path.as_path(), ix))
            .collect();
        let groups = parts
            .iter()
            .map(|part| {
                let mut held: Vec<PathBuf> = part
                    .files
                    .iter()
                    .chain(&part.joined)
                    .map(|&f| files[f].clone())
                    .collect();
                held.sort();
                let read: std::collections::HashSet<usize> = held
                    .iter()
                    .filter_map(|path| by_path.get(path.as_path()).copied())
                    .collect();
                let functions: Vec<usize> = (0..project.functions.len())
                    .filter(|&id| read.contains(&project.functions[id].file))
                    .collect();
                Group {
                    words: top_words(project, &functions, &held),
                    folder: folder_label(part),
                    members: members(part, &files),
                    functions: functions.len(),
                    files: held,
                }
            })
            .collect();
        Grouping { groups, calls }
    }

    /// The groups as draft domains: named by their words, with the calls
    /// between them as flows.
    pub fn domains(&self) -> Domains {
        let mut names: Vec<String> = Vec::new();
        for group in &self.groups {
            let mut name = group.words.clone();
            if names.contains(&name) {
                name = format!("{name} ({})", group.folder);
            }
            names.push(name);
        }
        Domains {
            domains: self
                .groups
                .iter()
                .zip(&names)
                .map(|(group, name)| Domain {
                    name: name.clone(),
                    purpose: String::new(),
                    offers: Vec::new(),
                    why: group.folder.clone(),
                    members: group.members.clone(),
                })
                .collect(),
            flows: self
                .strongest_calls()
                .into_iter()
                .map(|((from, to), count)| Flow {
                    from: names[from].clone(),
                    to: names[to].clone(),
                    label: format!("{count} call{}", if count == 1 { "" } else { "s" }),
                })
                .collect(),
            shared: Default::default(),
        }
    }

    /// The calls between groups worth a link: each group's strongest few
    /// either way, so the drafts aren't a web of every call.
    fn strongest_calls(&self) -> Vec<((usize, usize), usize)> {
        const PER_GROUP: usize = 2;
        let mut kept: Vec<((usize, usize), usize)> = Vec::new();
        for group in 0..self.groups.len() {
            let mut mine: Vec<&((usize, usize), usize)> = self
                .calls
                .iter()
                .filter(|((from, to), _)| *from == group || *to == group)
                .collect();
            mine.sort_by_key(|(pair, count)| (std::cmp::Reverse(*count), *pair));
            for &&call in mine.iter().take(PER_GROUP) {
                if !kept.contains(&call) {
                    kept.push(call);
                }
            }
        }
        kept.sort();
        kept
    }

    /// The grouping as Index Domains hands it over: every file by group,
    /// then the calls between groups.
    pub fn hint(&self) -> String {
        let mut out =
            String::from("# Jig's grouping (folders, calls and names; the words are guesses)\n");
        for (ix, group) in self.groups.iter().enumerate() {
            out.push_str(&format!(
                "\n## group {} — words: {} — {} · {} files, {} functions\n",
                ix + 1,
                group.words,
                group.folder,
                group.files.len(),
                group.functions
            ));
            for file in &group.files {
                out.push_str(&format!("- {}\n", place_of(file)));
            }
        }
        if !self.calls.is_empty() {
            out.push_str("\n## calls between groups (from → to: count)\n");
            for ((from, to), count) in &self.calls {
                out.push_str(&format!(
                    "- group {} → group {}: {count}\n",
                    from + 1,
                    to + 1
                ));
            }
        }
        out
    }
}

/// How much code a file is: its length.
fn weight(project: &Project, stamps: &HashMap<&Path, u64>, path: &Path) -> u64 {
    stamps
        .get(path)
        .copied()
        .or_else(|| {
            std::fs::metadata(project.root.join(path))
                .ok()
                .map(|m| m.len())
        })
        .unwrap_or(0)
        .max(1)
}

/// Down a chain of folders holding one folder and nothing else.
fn descend(mut part: Part, files: &[PathBuf]) -> Part {
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

/// Whether a folder part has more than one thing in it to split into.
fn opens(part: &Part, files: &[PathBuf]) -> bool {
    open_part(part.clone(), files).len() > 1
}

/// A folder's folders, each a part, and the files loose in it, one more.
fn open_part(part: Part, files: &[PathBuf]) -> Vec<Part> {
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
fn file_calls(project: &Project, files: &[PathBuf]) -> HashMap<(usize, usize), usize> {
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
fn join_loose_files(parts: &mut [Part], files: &[PathBuf], calls: &HashMap<(usize, usize), usize>) {
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
fn merge_smallest(
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
fn members(part: &Part, files: &[PathBuf]) -> Vec<String> {
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

fn folder_label(part: &Part) -> String {
    let dir = place_of(&part.dir);
    let dir = if dir.is_empty() { "the project" } else { &dir };
    if part.merged.is_empty() {
        dir.to_string()
    } else {
        format!("{dir} and more")
    }
}

/// The three words `functions`' names use most, past the ones every
/// project uses; without functions, the files' names.
fn top_words(project: &Project, functions: &[usize], files: &[PathBuf]) -> String {
    const COMMON: &[&str] = &[
        "get", "set", "new", "from", "with", "into", "make", "test", "tests", "show", "open",
        "this", "self", "default", "the", "for", "and", "all", "has", "can", "add", "fmt",
        "handle", "create", "update", "render", "value", "should", "when", "use", "data", "item",
        "list", "string", "type", "init", "main", "run", "read", "write", "build", "load", "save",
        "find", "check", "apply", "event", "context", "props", "state", "map", "key", "next",
        "its", "not", "are", "one", "out", "via", "but", "mod", "lib", "index", "impl", "drop",
        "clone", "eq", "from_str", "to", "is", "on", "of", "in", "at", "by", "or", "an", "it",
    ];
    let mut counts: HashMap<String, usize> = HashMap::new();
    let count = |counts: &mut HashMap<String, usize>, name: &str| {
        for word in words(name) {
            if word.len() >= 3 && !COMMON.contains(&word.as_str()) {
                *counts.entry(word).or_default() += 1;
            }
        }
    };
    for &id in functions {
        count(&mut counts, project.functions[id].short());
    }
    if counts.is_empty() {
        for file in files {
            if let Some(stem) = file.file_stem() {
                count(&mut counts, &stem.to_string_lossy());
            }
        }
    }
    let mut ranked: Vec<(String, usize)> = counts.into_iter().collect();
    ranked.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    let words: Vec<String> = ranked.into_iter().take(3).map(|(w, _)| w).collect();
    if words.is_empty() {
        "unnamed".into()
    } else {
        words.join(" · ")
    }
}

/// A name's words, lower case: `bookRoom`, `book_room` and `BookRoom` are
/// all "book" and "room".
fn words(name: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut last_lower = false;
    for c in name.chars() {
        let boundary = !c.is_alphanumeric() || (c.is_uppercase() && last_lower);
        if boundary && !word.is_empty() {
            words.push(std::mem::take(&mut word));
        }
        if c.is_alphanumeric() {
            word.extend(c.to_lowercase());
        }
        last_lower = c.is_lowercase() || c.is_ascii_digit();
    }
    if !word.is_empty() {
        words.push(word);
    }
    words
}

#[cfg(test)]
mod tests;
