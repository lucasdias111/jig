//! The project's business domains, as `.jig/map/domains.toml` keeps them:
//! each domain's name, plain-words purpose, what it offers people, why its
//! files belong together and its members (files, `folder/` for all under
//! it, `path#Type` for a type of its own); the flows between domains; and
//! the code used everywhere, in Shared. The file is committed, so it's
//! keyed by names and paths, never by positions.
//!
//! Every file has one home: the domain (or Shared) whose member names it
//! most closely: a type of its own over its file, a file over its folder,
//! a deeper folder over one further out. A file nothing names is Unsorted,
//! until Update Domains places it.
//!
//! `grouping` is Jig's own grouping, shown as drafts before Index Domains
//! has run; `level` lays the domains out; `actions` holds what the agent
//! is asked and reads back what it answers.

pub mod actions;
pub mod grouping;
pub mod level;
mod text;
pub mod tools;

use std::path::{Path, PathBuf};

/// Where the domains are kept, in the project.
pub const FILE: &str = ".jig/map/domains.toml";

#[derive(Clone, Debug, Default, PartialEq, serde::Deserialize)]
pub struct Domains {
    #[serde(default, rename = "domain")]
    pub domains: Vec<Domain>,
    #[serde(default, rename = "flow")]
    pub flows: Vec<Flow>,
    #[serde(default)]
    pub shared: Shared,
}

#[derive(Clone, Debug, Default, PartialEq, serde::Deserialize)]
pub struct Domain {
    pub name: String,
    #[serde(default)]
    pub purpose: String,
    /// What it offers people: screens, commands, jobs, emails.
    #[serde(default)]
    pub offers: Vec<String>,
    /// Why its members belong together.
    #[serde(default)]
    pub why: String,
    #[serde(default)]
    pub members: Vec<String>,
}

/// What goes from one domain to another, in a few plain words.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Deserialize)]
pub struct Flow {
    pub from: String,
    pub to: String,
    #[serde(default)]
    pub label: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Deserialize)]
pub struct Shared {
    #[serde(default)]
    pub members: Vec<String>,
}

/// Where a file or type lives.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Home {
    Domain(usize),
    Shared,
    Unsorted,
}

impl Domains {
    pub fn index_of(&self, name: &str) -> Option<usize> {
        self.domains.iter().position(|domain| domain.name == name)
    }

    /// The home of `place` (a file, or `path#Type`): whoever names it most
    /// closely.
    pub fn home_of(&self, place: &str) -> Home {
        let mut best: Option<(Closeness, Home)> = None;
        let homes = self
            .domains
            .iter()
            .enumerate()
            .map(|(ix, domain)| (Home::Domain(ix), &domain.members))
            .chain(std::iter::once((Home::Shared, &self.shared.members)));
        for (home, members) in homes {
            for member in members {
                if let Some(close) = closeness(member, place)
                    && best.is_none_or(|(was, _)| close > was)
                {
                    best = Some((close, home));
                }
            }
        }
        best.map_or(Home::Unsorted, |(_, home)| home)
    }

    /// Domain `ix` called `name`, its flows following it.
    pub fn rename(&mut self, ix: usize, name: &str) {
        let name = name.trim();
        if name.is_empty() || self.index_of(name).is_some_and(|at| at != ix) {
            return;
        }
        let old = std::mem::replace(&mut self.domains[ix].name, name.to_string());
        for flow in &mut self.flows {
            if flow.from == old {
                flow.from = name.to_string();
            }
            if flow.to == old {
                flow.to = name.to_string();
            }
        }
    }

    pub fn set_purpose(&mut self, ix: usize, purpose: &str) {
        self.domains[ix].purpose = purpose.trim().to_string();
    }

    /// What flows from domain `from` to `to`, in words: the flow between
    /// them relabelled, or a new one.
    pub fn set_flow(&mut self, from: &str, to: &str, label: &str) {
        let label = label.trim().to_string();
        match self.flows.iter_mut().find(|f| f.from == from && f.to == to) {
            Some(flow) => flow.label = label,
            None => self.flows.push(Flow {
                from: from.to_string(),
                to: to.to_string(),
                label,
            }),
        }
        self.tidy_flows();
    }

    /// `place` (a file or `path#Type`) moved to `home`: named there, and no
    /// longer named anywhere else. A folder holding it elsewhere still
    /// does, but the closer name wins.
    pub fn move_member(&mut self, place: &str, home: Home) {
        for domain in &mut self.domains {
            domain.members.retain(|member| member != place);
        }
        self.shared.members.retain(|member| member != place);
        if self.home_of(place) == home {
            return;
        }
        match home {
            Home::Domain(ix) => self.domains[ix].members.push(place.to_string()),
            Home::Shared => self.shared.members.push(place.to_string()),
            // Only a place nothing else names is unsorted; it stays where
            // its folder puts it otherwise.
            Home::Unsorted => {}
        }
    }

    /// Domain `from` merged into domain `into`: its members join, its flows
    /// go from and to `into`, and it's gone.
    pub fn merge(&mut self, from: usize, into: usize) {
        if from == into || from >= self.domains.len() || into >= self.domains.len() {
            return;
        }
        let gone = self.domains.remove(from);
        let into = if into > from { into - 1 } else { into };
        let name = self.domains[into].name.clone();
        for member in gone.members {
            if !self.domains[into].members.contains(&member) {
                self.domains[into].members.push(member);
            }
        }
        for offer in gone.offers {
            if !self.domains[into].offers.contains(&offer) {
                self.domains[into].offers.push(offer);
            }
        }
        for flow in &mut self.flows {
            if flow.from == gone.name {
                flow.from = name.clone();
            }
            if flow.to == gone.name {
                flow.to = name.clone();
            }
        }
        self.tidy_flows();
    }

    /// Flows within a domain, to a domain that isn't there, or repeating
    /// one already given, dropped.
    pub fn tidy_flows(&mut self) {
        let names: Vec<String> = self.domains.iter().map(|d| d.name.clone()).collect();
        let mut seen = Vec::new();
        self.flows.retain(|flow| {
            let pair = (flow.from.clone(), flow.to.clone());
            let keep = flow.from != flow.to
                && names.contains(&flow.from)
                && names.contains(&flow.to)
                && !seen.contains(&pair);
            seen.push(pair);
            keep
        });
    }
}

/// How closely a member names a place, closest highest: a type of its
/// own, then its file, then its folders, deeper first.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Closeness {
    Folder(usize),
    File,
    Type,
}

fn closeness(member: &str, place: &str) -> Option<Closeness> {
    let member = member.trim().trim_start_matches("./");
    let file = place.split('#').next().unwrap_or(place);
    if member == place && place.contains('#') {
        return Some(Closeness::Type);
    }
    if member.contains('#') {
        return None;
    }
    if member == file {
        return Some(Closeness::File);
    }
    if let Some(folder) = member.strip_suffix('/') {
        if folder.is_empty() || file.starts_with(member) {
            return Some(Closeness::Folder(member.len()));
        }
    } else if file.starts_with(&format!("{member}/")) {
        // A folder written without its slash.
        return Some(Closeness::Folder(member.len() + 1));
    }
    None
}

fn quoted(text: &str) -> String {
    toml::Value::String(text.to_string()).to_string()
}

fn inline_list(items: &[String]) -> String {
    let items: Vec<String> = items.iter().map(|item| quoted(item)).collect();
    format!("[{}]", items.join(", "))
}

fn long_list(items: &[String]) -> String {
    if items.is_empty() {
        return "[]".into();
    }
    let items: Vec<String> = items
        .iter()
        .map(|item| format!("  {}", quoted(item)))
        .collect();
    format!("[\n{}\n]", items.join(",\n"))
}

/// A path as domains name it: relative, with forward slashes.
pub fn place_of(path: &Path) -> String {
    super::plan::slashed(path)
}

/// Every file the Domain level places, relative: the files the index
/// read and those only placed by their folder, in path order.
pub fn all_files(project: &super::index::Project) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = project
        .files
        .iter()
        .map(|file| file.path.clone())
        .chain(project.others.iter().cloned())
        .collect();
    files.sort();
    files
}

#[cfg(test)]
mod tests;
