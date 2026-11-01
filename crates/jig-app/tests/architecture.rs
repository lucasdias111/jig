//! Keeps Jig's capabilities apart, as `architecture.toml` at the repository's
//! root lays them out: which Jig crates each crate depends on, and inside
//! jig-app, which capability may name which other's modules.
//!
//! The code is read as tokens rather than compiled, so this sees every
//! `crate::` and `super::` path, in `use` lines, code and macros alike, but
//! not names brought in some other way (a glob, or a path through a module
//! of the same capability that re-exports another's).
//!
//! Inside a crate whose capability is `layered`, [`layers`] keeps its
//! facade, `ui`, `domain` and `store` apart the same way.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use proc_macro2::{Delimiter, Spacing, TokenStream, TokenTree};
use serde::Deserialize;

#[path = "architecture/layers.rs"]
mod layers;
#[path = "architecture/scan_tests.rs"]
mod scan_tests;

#[path = "architecture/scan.rs"]
mod scan;
use scan::{
    Scan, after_item, follows_separator, is_cfg_test, is_separator, rest_of_path, source_files,
};

const BLESS: &str = "JIG_ARCH_BLESS";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Architecture {
    crates: BTreeMap<String, Vec<String>>,
    layers: BTreeMap<String, layers::Layer>,
    capability: BTreeMap<String, Capability>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Capability {
    /// The crate it lives in, when it has one of its own. Its contract is
    /// then what the crate makes public, which the compiler keeps to.
    #[serde(rename = "crate")]
    krate: Option<String>,
    #[serde(default)]
    modules: Vec<String>,
    #[serde(default)]
    contract: Vec<String>,
    uses: Vec<String>,
    /// Whether its crate is laid out in layers yet (`[layers]`).
    #[serde(default)]
    layered: bool,
}

impl Capability {
    /// Its crate as code names it: `jig_map` for `jig-map`.
    fn crate_name(&self) -> Option<String> {
        self.krate.as_ref().map(|name| name.replace('-', "_"))
    }
}

fn repository() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn architecture() -> Architecture {
    let text = fs::read_to_string(repository().join("architecture.toml")).unwrap();
    toml::from_str(&text).unwrap_or_else(|e| panic!("architecture.toml: {e}"))
}

#[test]
fn crates_depend_only_as_listed() {
    let architecture = architecture();
    let mut problems = Vec::new();
    let mut found = BTreeSet::new();
    for entry in fs::read_dir(repository().join("crates")).unwrap() {
        let manifest = entry.unwrap().path().join("Cargo.toml");
        let Ok(text) = fs::read_to_string(&manifest) else {
            continue;
        };
        let manifest: toml::Table = toml::from_str(&text).unwrap();
        let name = manifest["package"]["name"].as_str().unwrap().to_string();
        let Some(allowed) = architecture.crates.get(&name) else {
            problems.push(format!("{name} isn't in architecture.toml's [crates]"));
            continue;
        };
        for dependency in jig_dependencies(&manifest) {
            if !allowed.contains(&dependency) {
                problems.push(format!(
                    "{name} depends on {dependency}, which [crates] doesn't allow"
                ));
            }
        }
        found.insert(name);
    }
    for name in architecture.crates.keys() {
        if !found.contains(name) {
            problems.push(format!("[crates] lists {name}, which isn't a crate"));
        }
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

/// The Jig crates a manifest names, as dependencies of any kind.
fn jig_dependencies(manifest: &toml::Table) -> BTreeSet<String> {
    let mut tables: Vec<&toml::Table> = Vec::new();
    for kind in ["dependencies", "dev-dependencies", "build-dependencies"] {
        tables.extend(manifest.get(kind).and_then(|t| t.as_table()));
        if let Some(targets) = manifest.get("target").and_then(|t| t.as_table()) {
            for target in targets.values() {
                tables.extend(target.get(kind).and_then(|t| t.as_table()));
            }
        }
    }
    tables
        .into_iter()
        .flat_map(|table| table.keys())
        .filter(|name| name.starts_with("jig-"))
        .cloned()
        .collect()
}

#[test]
fn capabilities_cross_only_through_their_contracts() {
    let architecture = architecture();
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let files = source_files(&source);
    let crates: Vec<String> = architecture
        .capability
        .values()
        .filter_map(Capability::crate_name)
        .collect();

    let mut modules = BTreeSet::new();
    let mut references = Vec::new();
    for (file, here) in &files {
        modules.insert(here.clone());
        let tokens: TokenStream = fs::read_to_string(file)
            .unwrap()
            .parse()
            .unwrap_or_else(|e| panic!("{}: {e:?}", file.display()));
        let mut scan = Scan {
            root: here.is_empty(),
            crates: crates.clone(),
            ..Scan::default()
        };
        scan.tokens(tokens, here);
        modules.extend(scan.modules);
        references.extend(scan.paths.into_iter().map(|path| (here.clone(), path)));
    }

    let map = Map::new(&architecture, &modules);
    let mut crossings: BTreeMap<String, String> = BTreeMap::new();
    for (here, path) in &references {
        if let Some((key, why)) = map.crossing(here, path) {
            crossings.entry(key).or_insert(why);
        }
    }

    let baseline_path = repository().join("architecture-baseline.txt");
    if std::env::var_os(BLESS).is_some() {
        fs::write(&baseline_path, baseline_text(&crossings)).unwrap();
        return;
    }
    let baseline: BTreeSet<String> = fs::read_to_string(&baseline_path)
        .unwrap_or_default()
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(String::from)
        .collect();

    let mut report = String::new();
    for (key, why) in &crossings {
        if !baseline.contains(key) {
            writeln!(report, "  {key}: {why}").unwrap();
        }
    }
    if !report.is_empty() {
        report.insert_str(0, "New crossings between capabilities:\n");
    }
    let gone: Vec<_> = baseline
        .iter()
        .filter(|key| !crossings.contains_key(*key))
        .collect();
    if !gone.is_empty() {
        writeln!(
            report,
            "These crossings no longer happen; take them out of architecture-baseline.txt:"
        )
        .unwrap();
        for key in gone {
            writeln!(report, "  {key}").unwrap();
        }
    }
    assert!(
        report.is_empty(),
        "{report}\nGo through the capability's contract, or change architecture.toml. \
         To accept the list as it now is, run {BLESS}=1 cargo test -p jig-app --test architecture"
    );
}

fn baseline_text(crossings: &BTreeMap<String, String>) -> String {
    let mut text = String::from(
        "# Crossings between jig-app's capabilities that architecture.toml doesn't allow,\n\
         # kept until they're gone. Each line is `from module -> to module`.\n\
         # Written by JIG_ARCH_BLESS=1 cargo test -p jig-app --test architecture.\n",
    );
    for key in crossings.keys() {
        text.push_str(key);
        text.push('\n');
    }
    text
}

/// The capabilities, and what each module belongs to.
struct Map<'a> {
    architecture: &'a Architecture,
    owner: BTreeMap<String, &'a str>,
    modules: &'a BTreeSet<Vec<String>>,
    crates: BTreeSet<String>,
}

impl<'a> Map<'a> {
    fn new(architecture: &'a Architecture, modules: &'a BTreeSet<Vec<String>>) -> Self {
        let mut owner = BTreeMap::new();
        let mut crates = BTreeSet::new();
        let mut problems = Vec::new();
        for (name, capability) in &architecture.capability {
            let crate_name = capability.crate_name();
            for module in capability.modules.iter().chain(&crate_name) {
                if let Some(other) = owner.insert(module.clone(), name.as_str()) {
                    problems.push(format!("{module} is in both {other} and {name}"));
                }
            }
            if let Some(krate) = &capability.krate {
                if !architecture.crates.contains_key(krate) {
                    problems.push(format!("{name}'s crate {krate} isn't in [crates]"));
                }
                if !capability.modules.is_empty() || !capability.contract.is_empty() {
                    problems.push(format!(
                        "{name} has a crate of its own, so its modules and contract are the crate's"
                    ));
                }
            }
            crates.extend(crate_name);
        }
        let tops: BTreeSet<String> = modules
            .iter()
            .map(|m| m.first().cloned().unwrap_or_else(|| "main".into()))
            .collect();
        for top in &tops {
            if !owner.contains_key(top) {
                problems.push(format!("module {top} isn't in any capability"));
            }
        }
        for (name, capability) in &architecture.capability {
            for module in &capability.modules {
                if !tops.contains(module) {
                    problems.push(format!("{name} lists {module}, which isn't a module"));
                }
            }
            for used in &capability.uses {
                if !architecture.capability.contains_key(used) {
                    problems.push(format!("{name} uses {used}, which isn't a capability"));
                }
            }
            for path in &capability.contract {
                let segments: Vec<String> = path.split("::").map(String::from).collect();
                let known = path == "main" || modules.contains(&segments);
                if !known || owner.get(&segments[0]) != Some(&name.as_str()) {
                    problems.push(format!(
                        "{name}'s contract names {path}, a module not its own"
                    ));
                }
            }
        }
        if let Some(cycle) = cycle(architecture) {
            problems.push(format!("uses go round in a loop: {}", cycle.join(" -> ")));
        }
        assert!(
            problems.is_empty(),
            "architecture.toml:\n{}",
            problems.join("\n")
        );
        Map {
            architecture,
            owner,
            modules,
            crates,
        }
    }

    /// The deepest module along `path`, as `a::b`; `main` for the crate root.
    fn module_of(&self, path: &[String]) -> String {
        if let Some(first) = path.first()
            && self.crates.contains(first)
        {
            return first.clone();
        }
        let mut depth = 0;
        while depth < path.len() && self.modules.contains(&path[..=depth].to_vec()) {
            depth += 1;
        }
        if depth == 0 {
            "main".into()
        } else {
            path[..depth].join("::")
        }
    }

    /// The baseline key and the reason, if naming `path` from module `here`
    /// crosses into another capability other than through its contract.
    fn crossing(&self, here: &[String], path: &[String]) -> Option<(String, String)> {
        let from_module = if here.is_empty() {
            "main".to_string()
        } else {
            here.join("::")
        };
        let to_module = self.module_of(path);
        let from = self.owner[here.first().map_or("main", String::as_str)];
        let to = self.owner[to_module.split("::").next().unwrap()];
        if from == to {
            return None;
        }
        let key = format!("{from_module} -> {to_module}");
        let capability = &self.architecture.capability[from];
        if !capability.uses.iter().any(|used| used == to) {
            return Some((key, format!("{from} doesn't use {to}")));
        }
        let target = &self.architecture.capability[to];
        if target.krate.is_none() && !target.contract.contains(&to_module) {
            return Some((key, format!("{to_module} isn't in {to}'s contract")));
        }
        None
    }
}

/// A loop in the capabilities' `uses`, if there is one.
fn cycle(architecture: &Architecture) -> Option<Vec<String>> {
    fn visit<'a>(
        name: &'a str,
        architecture: &'a Architecture,
        path: &mut Vec<&'a str>,
        done: &mut BTreeSet<&'a str>,
    ) -> Option<Vec<String>> {
        if let Some(start) = path.iter().position(|n| *n == name) {
            let mut cycle: Vec<String> = path[start..].iter().map(|n| n.to_string()).collect();
            cycle.push(name.to_string());
            return Some(cycle);
        }
        if !done.insert(name) {
            return None;
        }
        path.push(name);
        for used in architecture
            .capability
            .get(name)
            .map_or(&[][..], |c| &c.uses)
        {
            if let Some(cycle) = visit(used, architecture, path, done) {
                return Some(cycle);
            }
        }
        path.pop();
        None
    }
    let mut done = BTreeSet::new();
    architecture
        .capability
        .keys()
        .find_map(|name| visit(name, architecture, &mut Vec::new(), &mut done))
}
