use super::super::super::index::tests::fixture;
use super::super::Home;
use super::*;

#[test]
fn words_split_every_way_names_are_written() {
    assert_eq!(words("bookRoom"), ["book", "room"]);
    assert_eq!(words("book_room"), ["book", "room"]);
    assert_eq!(words("BookRoom"), ["book", "room"]);
    assert_eq!(words("HTTPServer"), ["httpserver"]);
}

#[test]
fn groups_follow_folders_and_every_file_has_one() {
    let dir = fixture();
    std::fs::write(dir.path().join("client/src/App.svelte"), "<p>hi</p>").unwrap();
    let project = Project::read(dir.path());
    let grouping = Grouping::of(&project);
    let folders: Vec<&str> = grouping.groups.iter().map(|g| g.folder.as_str()).collect();
    // Two folders are too few: the biggest is opened.
    assert_eq!(
        folders,
        [
            "client/src",
            "server/src/org/demo/bugs",
            "server/src/org/demo/users"
        ]
    );
    let domains = grouping.domains();
    for file in super::super::all_files(&project) {
        let home = domains.home_of(&place_of(&file));
        assert!(
            matches!(home, Home::Domain(_)),
            "{} has no group",
            file.display()
        );
    }
    // Files the index can't read are handed over too.
    assert!(grouping.hint().contains("- client/src/App.svelte\n"));
    // The calls between groups are counted: BugService calls
    // UserService.
    assert_eq!(grouping.calls, [((1, 2), 1)]);
    assert_eq!(domains.flows[0].label, "1 call");
}

#[test]
fn the_folder_holding_most_is_split_and_loose_files_join_their_folder() {
    let dir = tempfile::tempdir().unwrap();
    let write = |name: &str, text: &str| {
        let path = dir.path().join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    };
    write(
        "app/src/git.rs",
        "pub fn stage_file() { crate::git::run::run_git(); }\n",
    );
    write("app/src/git/run.rs", "pub fn run_git() {}\n");
    write(
        "app/src/lsp/client.rs",
        "pub fn start_server() {}\npub fn stop_server() {}\n",
    );
    write(
        "app/src/main.rs",
        "fn main() { start_server(); start_server(); }\n",
    );
    write("tool/src/lib.rs", "pub fn tiny() {}\n");
    let project = Project::read(dir.path());
    let grouping = Grouping::of(&project);
    let git = grouping
        .groups
        .iter()
        .find(|group| group.folder == "app/src/git")
        .expect("the git folder is a group");
    // `git.rs` lies beside `git/`: it joins it.
    assert!(git.members.contains(&"app/src/git.rs".to_string()));
    // `main.rs` calls the server twice: it joins the lsp group.
    let lsp = grouping
        .groups
        .iter()
        .find(|group| group.folder == "app/src/lsp")
        .expect("the lsp folder is a group");
    assert!(
        lsp.files.contains(&PathBuf::from("app/src/main.rs")),
        "{grouping:#?}"
    );
    assert!(lsp.words.contains("server"));
}

/// Jig's grouping of a real project, printed: `JIG_MAP_PROJECT=<repo>
/// cargo test -p jig-app grouping_live -- --ignored --nocapture`, Jig
/// itself without it.
#[test]
#[ignore]
fn grouping_live() {
    let root = std::env::var_os("JIG_MAP_PROJECT")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.."));
    let started = std::time::Instant::now();
    let project = Project::read(&root);
    let read = started.elapsed();
    let started = std::time::Instant::now();
    let grouping = Grouping::of(&project);
    println!("read in {read:?}, grouped in {:?}", started.elapsed());
    // `JIG_INDEX_PROMPT=<file>`: what Index Domains would ask, written
    // there, to try on an agent by hand.
    if let Some(out) = std::env::var_os("JIG_INDEX_PROMPT") {
        let ask = super::super::actions::index_instruction(&grouping.hint());
        std::fs::write(out, ask).unwrap();
    }
    // `JIG_INDEX_REPLY=<file>`: an agent's answer, read as the Map would.
    if let Some(reply) = std::env::var_os("JIG_INDEX_REPLY") {
        let reply = std::fs::read_to_string(reply).unwrap();
        let domains = super::super::actions::Action::Index
            .proposal(&reply, None)
            .unwrap();
        let site = super::super::super::site::Site::of(&project, &domains, Default::default());
        let level = super::super::level::DomainLevel::of(&project, &domains, false, &site);
        for (_, region) in level.showing() {
            println!("{}: {}", region.name, region.detail());
        }
    }
    for group in &grouping.groups {
        println!(
            "{} — {} · {} files, {} functions",
            group.words,
            group.folder,
            group.files.len(),
            group.functions
        );
    }
}
