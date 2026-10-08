//! Each project's open files, remembered as tabs change and brought back
//! when its folder opens on an empty window (see [`crate::session`]).

use std::path::Path;

use gpui_kit::*;

use super::Workspace;
use super::tabs::canonical;
use crate::session::{self, Session};

impl Workspace {
    /// Remember this window's tabs for its project. Untitled tabs aren't
    /// kept; on the start page nothing is open.
    pub(super) fn remember_session(&self, cx: &mut Context<Self>) {
        let Some(root) = self.project_root(cx) else {
            return;
        };
        let mut files = Vec::new();
        let mut active = 0;
        if !self.home {
            for (ix, tab) in self.tabs.iter().enumerate() {
                let Some(path) = tab.document.path.as_deref() else {
                    continue;
                };
                if ix <= self.active {
                    active = files.len();
                }
                files.push(canonical(path));
            }
        }
        session::remember(Session::new(&canonical(&root), &files, active), cx);
    }

    /// Open the files `root` last had open, showing the one that was
    /// showing. Files gone since are skipped.
    pub(super) fn restore_session(
        &mut self,
        root: &Path,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(saved) = session::get(&canonical(root), cx) else {
            return;
        };
        let (files, active) = saved.existing_files();
        for file in &files {
            self.open_file(file, window, cx);
        }
        if let Some(ix) = files.get(active).and_then(|file| self.tab_for(file)) {
            self.activate(ix, window, cx);
        }
    }
}
