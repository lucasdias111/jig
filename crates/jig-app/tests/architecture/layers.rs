//! Keeps a layered crate's layers apart, as `[layers]` in
//! `architecture.toml` lays them out: each file belongs to the layer of
//! its folder under `src/` (`ui`, `domain`, `store`), or to the facade when
//! it sits at the top beside `lib.rs`. A layer names only the layers it
//! uses and nothing it forbids, and the facade stays thin.
//!
//! Read as tokens, like the rest of this test: `crate::` and `super::`
//! paths say where a file reaches, and every path's start says what it
//! names, so `use std::{fs, path::Path}` names `std::fs`. Test code is left
//! out.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;

use proc_macro2::{Delimiter, TokenStream, TokenTree};
use serde::Deserialize;

use super::{
    Scan, after_item, architecture, follows_separator, is_cfg_test, is_separator, repository,
    rest_of_path, source_files,
};

/// The layer of every file at the top of `src/`, `lib.rs` among them.
const FACADE: &str = "facade";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Layer {
    /// The other layers it may name.
    uses: Vec<String>,
    /// Paths it may not name, by their start: `std::fs`, `gpui_kit`.
    #[serde(default)]
    forbids: Vec<String>,
    /// Traits it may not implement, by name: `Render`.
    #[serde(default)]
    forbids_impls: Vec<String>,
    /// How long any one of its files may be.
    max_lines: Option<usize>,
}

#[test]
fn layered_crates_keep_their_layers() {
    let architecture = architecture();
    let layers = &architecture.layers;
    let mut problems = Vec::new();
    if !layers.contains_key(FACADE) {
        problems.push(format!("[layers] has no {FACADE}"));
    }
    for (name, layer) in layers {
        for used in &layer.uses {
            if !layers.contains_key(used) || used == FACADE {
                problems.push(format!(
                    "layer {name} uses {used}, which isn't a layer below it"
                ));
            }
        }
    }
    for capability in architecture.capability.values().filter(|c| c.layered) {
        let Some(krate) = &capability.krate else {
            problems.push("layered = true needs a crate of its own".into());
            continue;
        };
        let source = repository().join("crates").join(krate).join("src");
        let files = source_files(&source);
        let tops: BTreeSet<String> = files
            .iter()
            .filter_map(|(_, m)| m.first().cloned())
            .collect();
        for (file, here) in &files {
            let text = fs::read_to_string(file).unwrap();
            let name = file.strip_prefix(repository()).unwrap_or(file).display();
            for problem in check_file(layers, &text, here, &tops) {
                problems.push(format!("{name}: {problem}"));
            }
        }
    }
    assert!(
        problems.is_empty(),
        "Layers (see docs/layers.md):\n{}",
        problems.join("\n")
    );
}

/// The layer of the file at module path `here`: its first folder's, when
/// that's a layer, else the facade's.
fn layer_of<'a>(layers: &'a BTreeMap<String, Layer>, here: &[String]) -> &'a str {
    match here.first() {
        Some(first) if first != FACADE => layers
            .get_key_value(first)
            .map_or(FACADE, |(name, _)| name.as_str()),
        _ => FACADE,
    }
}

/// What's wrong with the file at module path `here`, its text `text`, in a
/// crate whose top-level modules are `tops`.
fn check_file(
    layers: &BTreeMap<String, Layer>,
    text: &str,
    here: &[String],
    tops: &BTreeSet<String>,
) -> Vec<String> {
    let layer_name = layer_of(layers, here);
    let layer = &layers[layer_name];
    let tokens: TokenStream = text
        .parse()
        .unwrap_or_else(|e| panic!("{}: {e:?}", here.join("::")));
    let mut problems = Vec::new();

    let mut scan = Scan {
        root: here.is_empty(),
        ..Scan::default()
    };
    scan.tokens(tokens.clone(), here);
    let mut reached = BTreeSet::new();
    for path in &scan.paths {
        let to = match path.first() {
            None => FACADE,
            Some(first) if tops.contains(first) => layer_of(layers, path),
            // Another crate's, named from the root without `crate::`.
            Some(_) => continue,
        };
        if to != layer_name && !layer.uses.iter().any(|used| used == to) {
            reached.insert(format!(
                "{layer_name} names crate::{}, in {to}, which it doesn't use",
                path.join("::")
            ));
        }
    }
    problems.extend(reached);

    let mut named = Vec::new();
    named_paths(tokens.clone(), &mut named);
    let mut forbidden = BTreeSet::new();
    for path in &named {
        for forbid in &layer.forbids {
            let forbid: Vec<&str> = forbid.split("::").collect();
            if path.len() >= forbid.len() && path.iter().zip(&forbid).all(|(a, b)| a == b) {
                forbidden.insert(format!(
                    "{layer_name} names {}, which it forbids",
                    forbid.join("::")
                ));
            }
        }
    }
    problems.extend(forbidden);

    for name in implemented(tokens) {
        if layer.forbids_impls.contains(&name) {
            problems.push(format!(
                "{layer_name} implements {name}; that belongs in ui"
            ));
        }
    }
    let lines = text.lines().count();
    if let Some(max) = layer.max_lines
        && lines > max
    {
        problems.push(format!(
            "{layer_name} has {lines} lines, over {max}; move the work into a layer"
        ));
    }
    problems
}

/// Every path the tokens name, from its first segment, as written: `use
/// std::{fs, path::Path}` names `std::fs` and `std::path::Path`, and `use
/// gpui_kit as g` names `gpui_kit`. Test code is left out.
fn named_paths(tokens: TokenStream, out: &mut Vec<Vec<String>>) {
    let tokens: Vec<TokenTree> = tokens.into_iter().collect();
    let mut at = 0;
    while at < tokens.len() {
        match &tokens[at] {
            TokenTree::Punct(p) if p.as_char() == '#' && is_cfg_test(tokens.get(at + 1)) => {
                at = after_item(&tokens, at + 2);
                continue;
            }
            TokenTree::Ident(word) if !follows_separator(&tokens, at) => {
                let used = at > 0 && matches!(&tokens[at - 1], TokenTree::Ident(u) if u == "use");
                if is_separator(&tokens, at + 1) {
                    at = rest_of_path(&tokens, at + 1, vec![word.to_string()], out);
                    continue;
                }
                if used {
                    out.push(vec![word.to_string()]);
                }
            }
            TokenTree::Group(group) => named_paths(group.stream(), out),
            _ => {}
        }
        at += 1;
    }
}

/// The traits the tokens implement, by name: `impl<T> gpui::Render for X`
/// gives `Render`. Test code is left out.
fn implemented(tokens: TokenStream) -> Vec<String> {
    let tokens: Vec<TokenTree> = tokens.into_iter().collect();
    let mut found = Vec::new();
    let mut at = 0;
    while at < tokens.len() {
        match &tokens[at] {
            TokenTree::Punct(p) if p.as_char() == '#' && is_cfg_test(tokens.get(at + 1)) => {
                at = after_item(&tokens, at + 2);
                continue;
            }
            TokenTree::Ident(word) if word == "impl" => {
                let mut depth = 0;
                let mut last = None;
                for token in &tokens[at + 1..] {
                    match token {
                        TokenTree::Punct(p) if p.as_char() == '<' => depth += 1,
                        TokenTree::Punct(p) if p.as_char() == '>' => depth -= 1,
                        TokenTree::Ident(word) if word == "for" && depth == 0 => {
                            found.extend(last.take());
                            break;
                        }
                        TokenTree::Ident(word) if depth == 0 => last = Some(word.to_string()),
                        TokenTree::Group(g) if g.delimiter() == Delimiter::Brace => break,
                        _ => {}
                    }
                }
            }
            TokenTree::Group(group) => found.extend(implemented(group.stream())),
            _ => {}
        }
        at += 1;
    }
    found
}

fn check(text: &str, here: &[&str]) -> Vec<String> {
    let here: Vec<String> = here.iter().map(|s| s.to_string()).collect();
    let tops = ["domain", "store", "ui", "icons"]
        .map(String::from)
        .into_iter()
        .collect();
    check_file(&architecture().layers, text, &here, &tops)
}

#[test]
fn keeps_io_and_gpui_out_of_the_domain() {
    assert_eq!(
        check(
            "use std::{fs, path::Path};\nuse gpui_kit::*;\nfn f() { jig_process::run(); }",
            &["domain", "links"]
        ),
        [
            "domain names gpui_kit, which it forbids",
            "domain names jig_process, which it forbids",
            "domain names std::fs, which it forbids",
        ]
    );
    assert!(check("use std::path::Path;\nuse gpui_kit as g;", &["ui", "form"]).is_empty());
    assert_eq!(
        check("use gpui_kit as g;", &["domain"]),
        ["domain names gpui_kit, which it forbids"]
    );
}

#[test]
fn lets_layers_name_only_what_they_use() {
    assert!(
        check(
            "use crate::domain::tasks::TaskStore;",
            &["store", "persist"]
        )
        .is_empty()
    );
    assert!(check("use crate::store::session::Board;", &["ui", "planner"]).is_empty());
    assert_eq!(
        check("use super::super::ui::TaskPlanner;", &["domain", "tasks"]),
        ["domain names crate::ui::TaskPlanner, in ui, which it doesn't use"]
    );
    assert_eq!(
        check(
            "fn f() { crate::ui::widgets::field(); }",
            &["store", "session"]
        ),
        ["store names crate::ui::widgets::field, in ui, which it doesn't use"]
    );
    assert_eq!(
        check("use crate::icons::PLUS;", &["domain"]),
        ["domain names crate::icons::PLUS, in facade, which it doesn't use"]
    );
    assert!(
        check(
            "pub use ui::TaskPlanner;\npub use domain::links::target;",
            &[]
        )
        .is_empty(),
        "the facade, from the root"
    );
}

#[test]
fn keeps_the_facade_thin() {
    assert_eq!(
        check("impl<T> gpui_kit::Render for Planner<T> {}", &[]),
        ["facade implements Render; that belongs in ui"]
    );
    assert!(check("impl From<Task> for Card {}", &[]).is_empty());
    assert_eq!(
        check("fn f() { std::fs::read(\"a\"); }", &["icons"]),
        ["facade names std::fs, which it forbids"]
    );
    let long = "const A: u8 = 1;\n".repeat(151);
    assert_eq!(
        check(&long, &[]),
        ["facade has 151 lines, over 150; move the work into a layer"]
    );
}

#[test]
fn leaves_test_code_out_of_the_layers() {
    assert!(
        check(
            "#[cfg(test)]\nmod tests { use std::fs; use crate::ui::X; }\nfn f() {}",
            &["domain", "tasks"]
        )
        .is_empty()
    );
}
