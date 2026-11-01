//! The file tree's actions and their keys, and the icons it draws.

use gpui_kit::*;

pub(super) const CONTEXT: &str = "FileTree";
/// The tree's context while a name is being typed, so its single-key
/// bindings (space, arrows) reach the name field instead.
pub(super) const NAMING_CONTEXT: &str = "FileTreeNaming";
pub(super) const ROW_HEIGHT: f32 = 26.;
pub(super) const INDENT: f32 = 14.;

pub(super) const CHEVRON_RIGHT: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="m9 18 6-6-6-6"/></svg>"#;
pub(super) const FILE_PLUS: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M15 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V7Z"/><path d="M14 2v4a2 2 0 0 0 2 2h4"/><path d="M9 15h6"/><path d="M12 18v-6"/></svg>"#;
pub(super) const FOLDER_PLUS: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M12 10v6"/><path d="M9 13h6"/><path d="M20 20a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.9L9.6 3.9A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2Z"/></svg>"#;
pub(super) const FOLDER: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="black"><path d="M3 6.5A2.5 2.5 0 0 1 5.5 4h3.88a2 2 0 0 1 1.42.59L12.2 6H18.5A2.5 2.5 0 0 1 21 8.5v9a2.5 2.5 0 0 1-2.5 2.5h-13A2.5 2.5 0 0 1 3 17.5z"/></svg>"#;
pub(super) const FILE: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="1.75" stroke-linejoin="round"><path d="M14 3H7a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V8z"/><path d="M14 3v5h5"/></svg>"#;
pub(super) const SEARCH: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="11" cy="11" r="7"/><path d="m20 20-3.5-3.5"/></svg>"#;
pub(super) const CHEVRON_DOWN: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="m6 9 6 6 6-6"/></svg>"#;

actions!(
    file_tree,
    [
        SelectPrevious,
        SelectNext,
        ExtendPrevious,
        ExtendNext,
        SelectAll,
        CollapseOrParent,
        ExpandOrChild,
        OpenSelected,
        Dismiss,
        CancelNaming,
        NewFileHere,
        NewFolderHere,
        RenameSelected,
        TrashSelected,
        CopyPath,
        CopyRelativePath,
        RevealSelected
    ]
);

pub fn key_bindings() -> Vec<KeyBinding> {
    vec![
        KeyBinding::new("up", SelectPrevious, Some(CONTEXT)),
        KeyBinding::new("down", SelectNext, Some(CONTEXT)),
        KeyBinding::new("shift-up", ExtendPrevious, Some(CONTEXT)),
        KeyBinding::new("shift-down", ExtendNext, Some(CONTEXT)),
        KeyBinding::new("secondary-a", SelectAll, Some(CONTEXT)),
        KeyBinding::new("left", CollapseOrParent, Some(CONTEXT)),
        KeyBinding::new("right", ExpandOrChild, Some(CONTEXT)),
        KeyBinding::new("enter", OpenSelected, Some(CONTEXT)),
        KeyBinding::new("space", OpenSelected, Some(CONTEXT)),
        KeyBinding::new("escape", Dismiss, Some(CONTEXT)),
        // Finder's keys for renaming and deleting are Enter and ⌘⌫, but
        // Enter opens here, so rename takes F2 as it does in the code.
        KeyBinding::new("f2", RenameSelected, Some(CONTEXT)),
        KeyBinding::new("secondary-backspace", TrashSelected, Some(CONTEXT)),
        KeyBinding::new("escape", CancelNaming, Some(NAMING_CONTEXT)),
    ]
}
