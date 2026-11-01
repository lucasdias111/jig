//! `map_project`: Jig's outline of the project, by group, folder or file.

use super::*;

/// `map_project`'s answer for `input`: the overview, a group's or a
/// folder's files in detail, or just paths, a page at a time.
pub fn project_outline(
    project: &Project,
    domains: Option<&Domains>,
    input: &Value,
) -> Result<String, String> {
    let grouping = Grouping::of(project);
    let files = source_files(project);
    let page = input["page"].as_u64().unwrap_or(1).max(1) as usize;
    let group = input["group"].as_u64().map(|group| group as usize);
    let folder = input["folder"]
        .as_str()
        .map(|folder| folder.trim().trim_start_matches("./").to_string())
        .filter(|folder| !folder.is_empty() && folder != "." && folder != "/");
    let files_only = input["files_only"].as_bool().unwrap_or(false);

    let scope: Vec<String> = match (group, &folder) {
        (Some(group), _) => {
            let Some(found) = group.checked_sub(1).and_then(|ix| grouping.groups.get(ix)) else {
                return Err(format!(
                    "There's no group {group}: the groups are numbered 1 to {}.",
                    grouping.groups.len()
                ));
            };
            found.files.iter().map(|path| place_of(path)).collect()
        }
        (None, Some(folder)) => {
            let prefix = format!("{}/", folder.trim_end_matches('/'));
            let held: Vec<String> = files
                .iter()
                .filter(|file| file.starts_with(&prefix) || *file == folder)
                .cloned()
                .collect();
            if held.is_empty() {
                return Err(format!(
                    "No source file is in {folder}. Name a folder relative to the project, as map_project's overview lists them."
                ));
            }
            held
        }
        (None, None) if files_only => files.clone(),
        (None, None) => return Ok(overview(project, &grouping, domains, &files)),
    };
    let size = if files_only { PATHS_PAGE } else { DETAIL_PAGE };
    let pages = scope.len().div_ceil(size).max(1);
    if page > pages {
        return Err(format!("There are only {pages} pages."));
    }
    let shown = &scope[(page - 1) * size..(page * size).min(scope.len())];
    let mut out = String::new();
    if files_only {
        for file in shown {
            out.push_str(&format!("- {file}\n"));
        }
    } else {
        let group_of = group_of(&grouping);
        for file in shown {
            out.push_str(&describe(
                project,
                file,
                group_of.get(file.as_str()).copied(),
            ));
        }
    }
    if pages > 1 {
        out.push_str(&format!(
            "\nPage {page} of {pages} ({} files).{}\n",
            scope.len(),
            if page < pages {
                format!(" Call again with \"page\": {} for more.", page + 1)
            } else {
                String::new()
            }
        ));
    }
    Ok(out)
}

/// What `map_project` says with nothing narrowed: the grouping, every
/// file by group while there aren't too many, and the calls between
/// groups.
pub(super) fn overview(
    project: &Project,
    grouping: &Grouping,
    domains: Option<&Domains>,
    files: &[String],
) -> String {
    let unread = project.others.len();
    let mut out = format!(
        "{} source files: {} read by Jig's index, {unread} it can't read (pages, templates, other languages; read those with your own tools). Every one of them is in a group below.\n",
        files.len(),
        files.len() - unread
    );
    if let Some(domains) = domains {
        let names: Vec<&str> = domains.domains.iter().map(|d| d.name.as_str()).collect();
        let unsorted = files
            .iter()
            .filter(|file| domains.home_of(file) == Home::Unsorted)
            .count();
        out.push_str(&format!(
            "Saved domains: {}; {unsorted} files in none of them.\n",
            names.join(", ")
        ));
    }
    out.push('\n');
    out.push_str(&grouping_hint(grouping));
    out.push_str(
        "\nFor each file's purpose, types, functions, imports and calls, call map_project with {\"group\": N} or {\"folder\": \"path/\"}; for just the paths, {\"files_only\": true}. Long answers come in pages: add {\"page\": 2}.\n",
    );
    out
}

/// Jig's grouping as the agent is handed it: every file by group, or
/// for a big project just the groups, and the calls between them.
pub fn grouping_hint(grouping: &Grouping) -> String {
    let files: usize = grouping.groups.iter().map(|group| group.files.len()).sum();
    if files <= OVERVIEW_FILES {
        return grouping.hint();
    }
    let mut out =
        String::from("# Jig's grouping (folders, calls and names; the words are guesses)\n");
    for (ix, group) in grouping.groups.iter().enumerate() {
        out.push_str(&format!(
            "- group {} — words: {} — {} · {} files, {} functions\n",
            ix + 1,
            group.words,
            group.folder,
            group.files.len(),
            group.functions
        ));
    }
    out.push_str("\nThere are too many files to list here: map_project lists each group's.\n");
    if !grouping.calls.is_empty() {
        out.push_str("\n## calls between groups (from → to: count)\n");
        for ((from, to), count) in &grouping.calls {
            out.push_str(&format!(
                "- group {} → group {}: {count}\n",
                from + 1,
                to + 1
            ));
        }
    }
    out
}

/// Each file's group, numbered from 1.
pub(super) fn group_of(grouping: &Grouping) -> HashMap<String, usize> {
    grouping
        .groups
        .iter()
        .enumerate()
        .flat_map(|(ix, group)| group.files.iter().map(move |file| (place_of(file), ix + 1)))
        .collect()
}

/// One file, as `map_project` describes it.
pub(super) fn describe(project: &Project, place: &str, group: Option<usize>) -> String {
    let group = group.map(|g| format!(" (group {g})")).unwrap_or_default();
    let Some(ix) = project
        .files
        .iter()
        .position(|file| place_of(&file.path) == place)
    else {
        return format!(
            "\n### {place}{group}\nNot read by Jig's index: read it yourself if its name doesn't say enough.\n"
        );
    };
    let file = &project.files[ix];
    let mut out = format!("\n### {place}{group}\n");
    if let Some(purpose) = &file.purpose {
        out.push_str(&format!("purpose: {purpose}\n"));
    }
    if !file.types.is_empty() {
        let types: Vec<String> = file
            .types
            .iter()
            .map(|ty| {
                let mut line = format!("{} {}", ty.kind, ty.name);
                if !ty.implements.is_empty() {
                    line.push_str(&format!(" ({})", ty.implements.join(", ")));
                }
                if let Some(purpose) = &ty.purpose {
                    line.push_str(&format!(": {purpose}"));
                }
                line
            })
            .collect();
        out.push_str(&format!("types: {}\n", types.join("; ")));
    }
    let functions: Vec<usize> = (0..project.functions.len())
        .filter(|&id| project.functions[id].file == ix)
        .collect();
    if !functions.is_empty() {
        let mut named: Vec<String> = functions
            .iter()
            .take(FUNCTIONS_SHOWN)
            .map(|&id| {
                let function = &project.functions[id];
                format!("{}{}", function.title(), signature(function))
            })
            .collect();
        if functions.len() > FUNCTIONS_SHOWN {
            named.push(format!("+{} more", functions.len() - FUNCTIONS_SHOWN));
        }
        out.push_str(&format!("functions: {}\n", named.join("; ")));
    }
    if !file.imports.is_empty() {
        let imports: Vec<&str> = file
            .imports
            .iter()
            .map(|import| import.path.as_str())
            .collect();
        out.push_str(&format!("imports: {}\n", imports.join(", ")));
    }
    let mut calls: HashMap<usize, usize> = HashMap::new();
    for &id in &functions {
        for edge in &project.edges[id] {
            if let Target::Function(to) = edge.target {
                let into = project.functions[to].file;
                if into != ix {
                    *calls.entry(into).or_default() += 1;
                }
            }
        }
    }
    if !calls.is_empty() {
        let mut calls: Vec<(usize, usize)> = calls.into_iter().collect();
        calls.sort_by_key(|&(file, count)| (std::cmp::Reverse(count), file));
        let calls: Vec<String> = calls
            .iter()
            .take(8)
            .map(|&(file, count)| format!("{} ({count})", place_of(&project.files[file].path)))
            .collect();
        out.push_str(&format!("calls into: {}\n", calls.join(", ")));
    }
    out
}
