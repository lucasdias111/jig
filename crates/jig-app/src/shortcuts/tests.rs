use std::collections::{BTreeMap, HashSet};

use super::{ALL, bindings};
use jig_settings::clashing_shortcuts as clashes;

#[test]
fn every_default_parses_and_ids_are_unique() {
    let ids: HashSet<_> = ALL.iter().map(|shortcut| shortcut.id).collect();
    assert_eq!(ids.len(), ALL.len());
    let expected: usize = ALL
        .iter()
        .map(|shortcut| shortcut.keys.len() * shortcut.contexts.len())
        .sum();
    assert_eq!(bindings(&BTreeMap::new()).len(), expected);
}

#[test]
fn defaults_dont_clash() {
    for shortcut in ALL {
        assert_eq!(
            clashes(shortcut, ALL, &BTreeMap::new()),
            Vec::<&str>::new(),
            "{}",
            shortcut.id
        );
    }
}

#[test]
fn a_change_replaces_the_defaults_and_empty_unbinds() {
    let save = ALL.iter().find(|shortcut| shortcut.id == "save").unwrap();
    let mut changed = BTreeMap::new();
    changed.insert("save".to_string(), "secondary-shift-d".to_string());
    assert_eq!(save.keys_in(&changed), ["secondary-shift-d"]);
    assert_eq!(clashes(save, ALL, &changed), ["Duplicate line"]);
    changed.insert("save".to_string(), String::new());
    assert!(save.keys_in(&changed).is_empty());
}
