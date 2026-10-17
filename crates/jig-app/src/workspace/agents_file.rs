//! Editing the project's `AGENTS.md`, the instructions the agent reads.

use gpui_kit::*;

use super::{EditAgentsFile, Workspace};
use crate::project;

impl Workspace {
    /// Open the `AGENTS.md` that applies to the current file, creating one
    /// at the project root if there is none.
    pub(super) fn edit_agents_file(
        &mut self,
        _: &EditAgentsFile,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(file) = self.document().path.clone() else {
            return self.show_error(
                "Save this file first, so Jig knows which project it belongs to.",
                window,
                cx,
            );
        };
        let path = project::agents_path(&file)
            .unwrap_or_else(|| project::root_for(&file).join(project::AGENTS_FILE));
        if !path.exists()
            && let Err(error) = std::fs::write(&path, project::AGENTS_TEMPLATE)
        {
            return self.show_error(
                &format!("Couldn't create {}: {error}", path.display()),
                window,
                cx,
            );
        }
        self.open_file(&path, window, cx);
    }
}
