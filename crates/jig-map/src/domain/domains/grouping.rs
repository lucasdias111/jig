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
mod labels;
mod parts;
use labels::*;
use parts::*;

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
        .or_else(|| project.other_lengths.get(path).copied())
        .unwrap_or(0)
        .max(1)
}

/// Whether a folder part has more than one thing in it to split into.
fn opens(part: &Part, files: &[PathBuf]) -> bool {
    open_part(part.clone(), files).len() > 1
}

#[cfg(test)]
mod tests;
