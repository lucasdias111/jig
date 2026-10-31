//! How [`Scan`] reads paths from tokens.

use super::Scan;

fn scan(source: &str, here: &[&str]) -> Vec<String> {
    let here: Vec<String> = here.iter().map(|s| s.to_string()).collect();
    let mut scan = Scan {
        root: here.is_empty(),
        crates: vec!["jig_map".into()],
        ..Scan::default()
    };
    scan.tokens(source.parse().unwrap(), &here);
    scan.paths.iter().map(|p| p.join("::")).collect()
}

#[test]
fn reads_paths_in_use_lines_and_code() {
    assert_eq!(
        scan(
            "use crate::{map::{self, view::Scene}, git};\nfn f() { crate::lsp::start::<T>(); }",
            &["planner"],
        ),
        ["map", "map::view::Scene", "git", "lsp::start"]
    );
}

#[test]
fn resolves_super_from_where_the_file_is() {
    assert_eq!(
        scan("use super::super::git::Repo;", &["map", "view", "scene"]),
        ["map::git::Repo"]
    );
}

#[test]
fn reads_paths_from_the_root_without_crate() {
    assert_eq!(
        scan("fn main() { workspace::open(); }", &[]),
        ["workspace::open"]
    );
}

#[test]
fn leaves_out_test_code() {
    assert_eq!(
        scan(
            "#[cfg(test)]\nmod tests { use crate::map::Hidden; }\nuse crate::git::Shown;",
            &["planner"]
        ),
        ["git::Shown"]
    );
}

#[test]
fn reads_paths_into_capability_crates() {
    assert_eq!(
        scan(
            "use jig_map::{Level, MapView};\nfn f() { other::jig_map::x(); }",
            &["workspace"]
        ),
        ["jig_map::Level", "jig_map::MapView"]
    );
}
