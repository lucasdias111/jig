//! Blueprints firing off while the program runs. Each block of a function
//! a blueprint shows gets a logpoint on its first line while debugging: the
//! debugger prints a message there and carries on without stopping. The
//! message names the place, and the blueprints showing it light it up.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};

use gpui_kit::*;

use super::Workspace;
use super::tabs::canonical;
use crate::blueprint::view::BlueprintView;

/// What each logpoint's message starts with, then its place's id. No
/// braces: LLDB reads those as expressions to print.
const MARKER: &str = "jig-fired ";

#[derive(Default)]
pub(super) struct LivePoints {
    /// Each place a logpoint was set, by the id its message carries.
    places: Vec<(PathBuf, u32)>,
    ids: HashMap<(PathBuf, u32), usize>,
    /// The lines (0-based) the blueprints want to hear about, by file.
    lines: BTreeMap<PathBuf, BTreeSet<u32>>,
    /// The blueprints showing, by file, to light up.
    views: Vec<(PathBuf, WeakEntity<BlueprintView>)>,
}

impl LivePoints {
    /// Logpoints for `path`, as lines and messages, but not where a
    /// breakpoint already is.
    pub(super) fn logpoints(&mut self, path: &Path, breakpoints: &[u32]) -> Vec<(u32, String)> {
        let path = canonical(path);
        let Some(lines) = self.lines.get(&path).cloned() else {
            return Vec::new();
        };
        lines
            .into_iter()
            .filter(|line| !breakpoints.contains(line))
            .map(|line| {
                let next = self.places.len();
                let id = *self.ids.entry((path.clone(), line)).or_insert(next);
                if id == next {
                    self.places.push((path.clone(), line));
                }
                (line, format!("{MARKER}{id}"))
            })
            .collect()
    }

    /// The files with logpoints.
    pub(super) fn files(&self) -> impl Iterator<Item = &PathBuf> {
        self.lines.keys()
    }
}

/// The places' ids in a debugger's output, when it's our logpoints'. Some
/// debuggers put something before the message, as Delve does its
/// goroutine: `> [Go 1]: jig-fired 3`.
pub(super) fn fired(output: &str) -> Option<Vec<usize>> {
    output
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| line.trim().rsplit_once(MARKER)?.1.parse().ok())
        .collect::<Option<Vec<usize>>>()
        .filter(|ids| !ids.is_empty())
}

impl Workspace {
    /// Ask the debugger to report the lines the blueprints showing want,
    /// as they now are.
    pub(super) fn sync_live_points(&mut self, cx: &App) {
        let mut lines: BTreeMap<PathBuf, BTreeSet<u32>> = BTreeMap::new();
        let mut views = Vec::new();
        for tab in &self.tabs {
            let (Some(open), Some(path)) = (&tab.blueprint, &tab.document.path) else {
                continue;
            };
            let path = canonical(path);
            let wanted = open.view.read(cx).live_lines();
            if !wanted.is_empty() {
                lines.entry(path.clone()).or_default().extend(wanted);
                views.push((path, open.view.downgrade()));
            }
        }
        self.live.views = views;
        if lines == self.live.lines {
            return;
        }
        let changed: BTreeSet<PathBuf> = lines
            .keys()
            .chain(self.live.lines.keys())
            .filter(|path| lines.get(*path) != self.live.lines.get(*path))
            .cloned()
            .collect();
        self.live.lines = lines;
        for path in changed {
            self.breakpoints_changed(&path, cx);
        }
    }

    /// The program reached the logpoints `ids`: light up their blocks.
    pub(super) fn on_fired(&mut self, ids: &[usize], cx: &mut App) {
        for &id in ids {
            let Some((path, line)) = self.live.places.get(id) else {
                continue;
            };
            for (file, view) in &self.live.views {
                if file == path {
                    let line = *line;
                    let _ = view.update(cx, |view, cx| view.fire_line(line, cx));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::path::PathBuf;

    use super::{LivePoints, fired};

    #[test]
    fn reads_only_our_messages() {
        assert_eq!(fired("jig-fired 3\n"), Some(vec![3]));
        assert_eq!(fired("> [Go 1]: jig-fired 3\n"), Some(vec![3]));
        assert_eq!(fired("jig-fired 3\njig-fired 4\n"), Some(vec![3, 4]));
        assert_eq!(fired("total 6\n"), None);
        assert_eq!(fired("jig-fired 3\ntotal 6\n"), None);
        assert_eq!(fired(""), None);
    }

    #[test]
    fn logpoints_keep_their_ids_and_leave_breakpoints_be() {
        let path = PathBuf::from("/nowhere/main.rs");
        let mut live = LivePoints::default();
        live.lines.insert(path.clone(), BTreeSet::from([1, 4, 7]));
        assert_eq!(
            live.logpoints(&path, &[4]),
            [
                (1, "jig-fired 0".to_string()),
                (7, "jig-fired 1".to_string())
            ]
        );
        live.lines.insert(path.clone(), BTreeSet::from([4, 7]));
        assert_eq!(
            live.logpoints(&path, &[]),
            [
                (4, "jig-fired 2".to_string()),
                (7, "jig-fired 1".to_string())
            ]
        );
        assert_eq!(live.places[2], (path, 4));
    }
}
