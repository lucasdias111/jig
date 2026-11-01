use super::super::index::tests::fixture;
use super::*;

fn titles(site: &Site, ix: usize) -> Vec<&str> {
    site.nodes[ix]
        .children
        .iter()
        .map(|&c| site.nodes[c].title.as_str())
        .collect()
}

fn domains() -> Domains {
    toml::from_str(
        r#"
[[domain]]
name = "Bug Reports"
purpose = "Report what's broken."
members = ["server/src/org/demo/bugs/", "client/src/rooms.ts"]

[[domain]]
name = "People"
purpose = "Who uses it."
members = ["server/src/org/demo/users/"]

[shared]
members = ["client/src/net/"]
"#,
    )
    .unwrap()
}

#[test]
fn each_region_holds_its_share_of_each_component() {
    let dir = fixture();
    std::fs::write(
        dir.path().join("client/src/App.svelte"),
        "<script></script>",
    )
    .unwrap();
    let project = Project::read(dir.path());
    let site = Site::of(&project, &domains(), Show::default());
    assert_eq!(
        titles(&site, 0),
        ["Bug Reports", "People", "Shared", "Unsorted"]
    );
    let bugs = site.index_of("domain:Bug Reports").unwrap();
    // Its slice of both components, the server's chain folded away.
    assert_eq!(titles(&site, bugs), ["client", "server › bugs"]);
    let server = site.nodes[bugs].children[1];
    assert_eq!(
        titles(&site, server),
        ["BugService.java", "MailNotifier.java", "Notifier.java"]
    );
    // A file the index can't read is in the region its folder is in.
    let unsorted = site.index_of("domain:/unsorted").unwrap();
    assert!(site.holds(unsorted, site.box_of("client/src/App.svelte").unwrap()));
    // Everything inside the box it's in, and nothing overlapping.
    for (ix, node) in site.nodes.iter().enumerate() {
        if let Some(parent) = node.parent {
            let outer = site.nodes[parent].rect;
            assert_eq!(outer.union(&node.rect), outer, "{} sticks out", node.key);
        }
        for &other in &node.children {
            for &next in &node.children {
                if other < next {
                    assert!(
                        !site.nodes[other].rect.overlaps(&site.nodes[next].rect),
                        "{} and {} overlap in {ix}",
                        site.nodes[other].key,
                        site.nodes[next].key
                    );
                }
            }
        }
    }
}

#[test]
fn tests_show_only_when_asked() {
    let dir = fixture();
    let test = dir
        .path()
        .join("server/src/test/org/demo/bugs/BugServiceTest.java");
    std::fs::create_dir_all(test.parent().unwrap()).unwrap();
    std::fs::write(&test, "class BugServiceTest { void makes() {} }\n").unwrap();
    let project = Project::read(dir.path());
    let place = "file:server/src/test/org/demo/bugs/BugServiceTest.java";
    let site = Site::of(&project, &domains(), Show::default());
    assert!(site.index_of(place).is_none());
    let tests = Show { tests: true };
    assert!(
        Site::of(&project, &domains(), tests)
            .index_of(place)
            .is_some()
    );
}

#[test]
fn arrows_join_boxes_side_by_side_and_never_cross_regions() {
    let dir = fixture();
    let project = Project::read(dir.path());
    let site = Site::of(&project, &domains(), Show::default());
    // Everything open: BugService calls into People, but that's for
    // the domains' link to say, not an arrow.
    let open = site.shown_as(|_| true);
    let arrows = site.arrows(&open);
    assert!(
        arrows
            .iter()
            .all(|a| site.nodes[a.from].region == site.nodes[a.to].region)
    );
    // Calls inside a region join the boxes side by side.
    assert!(
        arrows
            .iter()
            .all(|a| site.nodes[a.from].parent == site.nodes[a.to].parent)
    );
    // Regions closed: nothing.
    let closed = site.shown_as(|ix| ix == 0);
    assert!(site.arrows(&closed).is_empty());
}

#[test]
fn calls_inside_a_folder_join_its_boxes_not_the_deep_ones() {
    let dir = fixture();
    let project = Project::read(dir.path());
    // Everything in one region.
    let site = Site::of(&project, &Domains::default(), Show::default());
    let server = site.box_of("server/src/").unwrap();
    let client = site.box_of("client/src/").unwrap();
    let service = site
        .box_of("server/src/org/demo/bugs/BugService.java")
        .unwrap();
    let open = site.shown_as(|_| true);
    let arrows = site.arrows(&open);
    // BugService → UserService: both inside `server`, between its
    // folders `bugs` and `users`.
    let bugs = site.nodes[service].parent.unwrap();
    let users = site.box_of("server/src/org/demo/users/").unwrap();
    assert!(arrows.iter().any(|a| a.from == bugs && a.to == users));
    assert!(
        arrows
            .iter()
            .filter(|a| a.from == service)
            .all(|a| site.nodes[a.to].parent == Some(bugs))
    );
    assert!(!arrows.iter().any(|a| a.from == server || a.from == client));
}

#[test]
fn places_land_on_the_box_holding_them() {
    let dir = fixture();
    let project = Project::read(dir.path());
    let site = Site::of(&project, &Domains::default(), Show::default());
    let file = site.box_of("client/src/rooms.ts").unwrap();
    assert_eq!(site.nodes[file].title, "rooms.ts");
    let users = site.box_of("server/src/org/demo/users/").unwrap();
    assert!(site.nodes[users].title.contains("UserService.java"));
    // A Java file holds one type: one box, the type's.
    assert!(
        site.box_of("server/src/org/demo/bugs/Notifier.java#Notifier")
            .is_some_and(|ix| site.nodes[ix].kind == Kind::File { read: true })
    );
}

#[test]
fn boxes_open_once_their_insides_read() {
    let dir = fixture();
    let project = Project::read(dir.path());
    let site = Site::of(&project, &Domains::default(), Show::default());
    let region = site.regions().next().unwrap();
    let node = &site.nodes[region];
    assert_eq!(node.opening(node.open_at * 0.99), 0.);
    assert_eq!(node.opening(node.open_to() * 1.01), 1.);
    for &child in &node.children {
        let typical = site.nodes[child].rect.width * node.open_at;
        assert!(typical >= READ * 0.5, "{typical}");
    }
}

/// The Map on a big project stays smooth: `JIG_MAP_PROJECT=<a repo of
/// about 10k files> cargo test -p jig-app big_project_live -- --ignored
/// --nocapture` (budgets are for a release build; a debug build gets
/// eight times as long). Reading it, reading it again unchanged,
/// laying Site out, what each frame at Site works out, and a Plan.
#[test]
#[ignore]
fn big_project_live() {
    use std::time::{Duration, Instant};
    let root = std::env::var_os("JIG_MAP_PROJECT").expect("JIG_MAP_PROJECT");
    let root = Path::new(&root);
    let slack = if cfg!(debug_assertions) { 8 } else { 1 };
    let within = |what: &str, took: Duration, budget_ms: u64| {
        println!("{what}: {took:?}");
        assert!(
            took <= Duration::from_millis(budget_ms * slack),
            "{what} took {took:?}, over {budget_ms} ms"
        );
    };
    let started = Instant::now();
    let project = Project::read(root);
    within("read", started.elapsed(), 20_000);
    println!(
        "{} files read, {} placed by folder, {} functions, {} skipped",
        project.files.len(),
        project.others.len(),
        project.functions.len(),
        project.skipped
    );
    let started = Instant::now();
    let again = Project::read_again(root, Some(&project));
    within("read again, unchanged", started.elapsed(), 4_000);
    assert_eq!(again.functions.len(), project.functions.len());
    let started = Instant::now();
    let grouping = super::super::domains::grouping::Grouping::of(&project);
    let site = Site::of(&project, &grouping.domains(), Show::default());
    within("grouped, and Site laid out", started.elapsed(), 4_000);
    println!(
        "{} boxes, {} counted calls",
        site.nodes.len(),
        site.calls.len()
    );
    // Each frame at Site: which boxes show, and the arrows between them,
    // all the way out and further in.
    let fit = 1000. / site.nodes[0].rect.width.max(1.);
    for zoom in [fit, fit * 4., fit * 16., fit * 64.] {
        let started = Instant::now();
        let shown = site.shown_as(|ix| site.nodes[ix].opening(zoom) >= 0.5);
        let arrows = site.arrows(&shown);
        within(
            &format!("a frame at {zoom:.3}x ({} arrows)", arrows.len()),
            started.elapsed(),
            30,
        );
    }
    // A Plan around the most called function.
    let busiest = (0..project.functions.len())
        .max_by_key(|&id| project.callers[id].len())
        .expect("a function");
    let started = Instant::now();
    let plan = super::super::plan::neighbourhood(
        &project,
        busiest,
        &super::super::plan::Opened::default(),
    );
    super::super::layout::layout(&plan);
    within(
        &format!("the Plan of {}", project.functions[busiest].title()),
        started.elapsed(),
        30,
    );
    // Seven callers and "+N more", and its tests folded into one.
    let callers = plan.nodes.iter().filter(|n| n.column == -1).count();
    assert!(
        callers <= super::super::plan::MAX_SIDE + 1,
        "{callers} callers"
    );
}
