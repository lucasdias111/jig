use std::fs;
use std::path::Path;

use super::{
    Moved, NewEntry, TreeModel, create_entry, move_entries, moved_path, outermost, rename_entry,
    stem_range,
};

fn names(model: &TreeModel) -> Vec<String> {
    model
        .rows()
        .iter()
        .map(|row| format!("{}{}", "  ".repeat(row.depth), row.name))
        .collect()
}

fn project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::create_dir_all(root.join(".git")).unwrap();
    fs::create_dir_all(root.join("src/ui")).unwrap();
    fs::create_dir_all(root.join("target/debug")).unwrap();
    fs::create_dir_all(root.join("empty")).unwrap();
    fs::write(root.join(".gitignore"), "target/\n*.log\n").unwrap();
    fs::write(root.join("Cargo.toml"), "").unwrap();
    fs::write(root.join("build.log"), "").unwrap();
    fs::write(root.join("README.md"), "").unwrap();
    fs::write(root.join("src/main.rs"), "").unwrap();
    fs::write(root.join("src/ui/tree.rs"), "").unwrap();
    fs::write(root.join("target/debug/jig"), "").unwrap();
    dir
}

#[test]
fn folders_first_then_files_without_git() {
    let dir = project();
    let model = TreeModel::new(dir.path());
    assert_eq!(
        names(&model),
        [
            "empty",
            "src",
            "target",
            ".gitignore",
            "build.log",
            "Cargo.toml",
            "README.md"
        ]
    );
    let empty = &model.rows()[0];
    assert!(empty.is_dir, "an empty folder is still a folder");
}

#[test]
fn expanding_reads_nested_folders() {
    let dir = project();
    let mut model = TreeModel::new(dir.path());
    let src = model.rows()[1].path.clone();
    model.toggle(&src);
    model.toggle(&src.join("ui"));
    assert_eq!(
        names(&model)[..5],
        ["empty", "src", "  ui", "    tree.rs", "  main.rs"]
    );
    model.toggle(&src);
    assert_eq!(names(&model).len(), 7, "collapsing hides the subtree");
    model.toggle(&src);
    assert!(
        names(&model).contains(&"    tree.rs".to_string()),
        "nested folders stay open"
    );
}

#[test]
fn ignored_entries_are_dimmed_down_the_tree() {
    let dir = project();
    let mut model = TreeModel::new(dir.path());
    let ignored = |model: &TreeModel, name: &str| {
        model
            .rows()
            .iter()
            .find(|row| row.name == name)
            .unwrap()
            .ignored
    };
    assert!(ignored(&model, "target"));
    assert!(ignored(&model, "build.log"));
    assert!(!ignored(&model, "src"));
    let target = model.root().join("target");
    model.toggle(&target);
    assert!(ignored(&model, "debug"), "inside an ignored folder");
}

#[test]
fn reveal_expands_ancestors() {
    let dir = project();
    let mut model = TreeModel::new(dir.path());
    let ix = model.reveal(&dir.path().join("src/ui/tree.rs")).unwrap();
    assert_eq!(model.rows()[ix].name, "tree.rs");
    assert_eq!(model.rows()[ix].depth, 2);
    assert_eq!(model.reveal(Path::new("/elsewhere/x.rs")), None);
}

#[test]
fn creates_files_and_folders() {
    let dir = project();
    let root = dir.path();
    let file = create_entry(root, " notes.md ", NewEntry::File).unwrap();
    assert_eq!(file, root.join("notes.md"));
    assert!(file.is_file());
    let nested = create_entry(root, "src/ui/tabs.rs", NewEntry::File).unwrap();
    assert!(nested.is_file(), "folders on the way are created");
    assert!(
        create_entry(root, "docs/guides", NewEntry::Folder)
            .unwrap()
            .is_dir()
    );

    assert_eq!(
        create_entry(root, "notes.md", NewEntry::File),
        Err("notes.md already exists.".into())
    );
    assert!(create_entry(root, "  ", NewEntry::File).is_err());
    assert!(create_entry(root, "../out.rs", NewEntry::File).is_err());
    assert!(create_entry(root, "/tmp/out.rs", NewEntry::Folder).is_err());
}

#[test]
fn renames_in_place() {
    let dir = project();
    let root = dir.path();
    let renamed = rename_entry(&root.join("README.md"), " NOTES.md ").unwrap();
    assert_eq!(renamed, root.join("NOTES.md"));
    assert!(renamed.is_file());
    let src = rename_entry(&root.join("src"), "lib").unwrap();
    assert!(
        src.join("ui/tree.rs").is_file(),
        "a folder takes its contents"
    );
    assert_eq!(
        rename_entry(&root.join("lib"), "lib").unwrap(),
        root.join("lib"),
        "the same name changes nothing"
    );
    let case = rename_entry(&root.join("NOTES.md"), "notes.md").unwrap();
    assert_eq!(case, root.join("notes.md"), "case alone may change");

    assert_eq!(
        rename_entry(&root.join("notes.md"), "Cargo.toml"),
        Err("Cargo.toml already exists.".into())
    );
    assert!(rename_entry(&root.join("notes.md"), "lib/notes.md").is_err());
    assert!(rename_entry(&root.join("notes.md"), "..").is_err());
    assert!(rename_entry(&root.join("notes.md"), "").is_err());
}

#[test]
fn paths_inside_a_renamed_folder_follow_it() {
    let (from, to) = (Path::new("/p/src"), Path::new("/p/lib"));
    assert_eq!(
        moved_path(Path::new("/p/src/ui/a.rs"), from, to),
        Some("/p/lib/ui/a.rs".into())
    );
    assert_eq!(moved_path(from, from, to), Some(to.into()));
    assert_eq!(moved_path(Path::new("/p/srcs/a.rs"), from, to), None);
}

#[test]
fn moves_entries_into_a_folder() {
    let dir = project();
    let root = dir.path();
    let (readme, ui, empty) = (
        root.join("README.md"),
        root.join("src/ui"),
        root.join("empty"),
    );
    let moved = move_entries(&[readme.clone(), ui.clone()], &empty);
    assert_eq!(moved.error, None);
    assert_eq!(
        moved.moves,
        [(readme, empty.join("README.md")), (ui, empty.join("ui"))]
    );
    assert!(
        empty.join("ui/tree.rs").is_file(),
        "a folder takes its contents"
    );

    let (src, main) = (root.join("src"), root.join("src/main.rs"));
    assert_eq!(
        move_entries(&[main.clone(), src.clone()], &src),
        Moved::default(),
        "what's already there, and the folder itself, stay put"
    );
    assert_eq!(
        move_entries(std::slice::from_ref(&empty), &empty.join("ui")).error,
        Some("empty can't go inside itself.".into())
    );
    fs::write(root.join("main.rs"), "").unwrap();
    let taken = move_entries(&[empty.join("README.md"), main.clone()], root);
    let root_name = root.file_name().unwrap().to_string_lossy();
    assert_eq!(
        taken.error,
        Some(format!("main.rs already exists in {root_name}."))
    );
    assert!(taken.moves.is_empty() && main.is_file(), "nothing moves");
    assert!(empty.join("README.md").is_file(), "not even the free names");
}

#[test]
fn entries_inside_a_moved_folder_go_with_it() {
    let paths = ["/p/src".into(), "/p/src/a.rs".into(), "/p/b.rs".into()];
    assert_eq!(
        outermost(&paths),
        [Path::new("/p/src"), Path::new("/p/b.rs")]
    );
}

#[test]
fn rename_selects_the_name_before_its_extension() {
    assert_eq!(stem_range("tree.rs", false), 0..4);
    assert_eq!(stem_range("app.test.ts", false), 0..8);
    assert_eq!(stem_range(".gitignore", false), 0..10);
    assert_eq!(stem_range("v1.2", true), 0..4);
}

#[test]
fn refresh_picks_up_changes_on_disk() {
    let dir = project();
    let mut model = TreeModel::new(dir.path());
    fs::write(dir.path().join("new.rs"), "").unwrap();
    fs::remove_dir(dir.path().join("empty")).unwrap();
    assert!(!names(&model).contains(&"new.rs".to_string()));
    model.refresh();
    assert!(names(&model).contains(&"new.rs".to_string()));
    assert!(!names(&model).contains(&"empty".to_string()));
}
