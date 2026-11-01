//! Sketches matched to code as it's read again, archived, and made into tasks.

use super::*;

/// A sketch saved by hand at save's Plan: `check` (real code), `Cache`
/// (none yet), and an arrow from save to check.
fn a_saved_sketch(dir: &tempfile::TempDir) {
    let folder = dir.path().join(".jig/map/sketches");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(
        folder.join("caching.toml"),
        r#"title = "Caching"
level = "plan:src/store.rs#save"

[[box]]
id = "check"
label = "check"
near = "src/store.rs#save"
at = [0.0, 200.0]
size = [180.0, 64.0]

[[box]]
id = "cache"
label = "Cache"
near = "src/store.rs#save"
at = [260.0, 200.0]
size = [180.0, 64.0]

[[arrow]]
from = "src/store.rs#save"
to = "check"

[[arrow]]
from = "check"
to = "cache"
"#,
    )
    .unwrap();
}

fn read_again(cx: &mut TestAppContext, view: &Entity<MapView>, dir: &tempfile::TempDir) {
    let root = dir.path().to_path_buf();
    view.update(cx, |view, cx| {
        let before = view.project().cloned();
        let project = jig_map::Project::read_again(&root, before.as_deref());
        view.set_project(std::sync::Arc::new(project), cx);
    });
}

#[gpui_kit::test]
fn boxes_turn_solid_as_their_code_appears_and_then_it_archives(cx: &mut TestAppContext) {
    let (dir, window, _workspace, view) = on_the_map(cx);
    a_saved_sketch(&dir);
    read_again(cx, &view, &dir);
    assert_eq!(
        cx.update(|cx| view.read(cx).match_of("caching", "check")),
        Match::Real("src/store.rs#check".into())
    );
    assert_eq!(
        cx.update(|cx| view.read(cx).match_of("caching", "cache")),
        Match::None
    );
    assert_eq!(cx.update(|cx| view.read(cx).real_boxes("caching")), (1, 2));
    assert!(!cx.update(|cx| view.read(cx).sketch_done("caching")));
    // The code appears, and calls along the arrow.
    std::fs::write(
        dir.path().join("src/cache.rs"),
        "pub struct Cache;\n\nimpl Cache {\n    pub fn keep() {}\n}\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("src/store.rs"),
        STORE.replace(
            "fn check(item: u8) {}",
            "fn check(item: u8) {\n    Cache::keep();\n}",
        ),
    )
    .unwrap();
    read_again(cx, &view, &dir);
    assert_eq!(
        cx.update(|cx| view.read(cx).match_of("caching", "cache")),
        Match::Real("src/cache.rs#Cache".into())
    );
    assert!(cx.update(|cx| view.read(cx).sketch_done("caching")));
    draw(cx, window);
    click_on(cx, window, "map-sketch-archive");
    assert!(cx.update(|cx| view.read(cx).sketches_showing().is_empty()));
    assert!(sketch_file(&dir, "caching").contains("archived = true"));
    // Still in the Sketches list, to restore.
    click_on(cx, window, "map-sketches");
    click_on(cx, window, "map-sketch-restore-0");
    assert_eq!(
        cx.update(|cx| view.read(cx).sketches_showing()),
        ["caching"]
    );
}

#[gpui_kit::test]
fn an_unsure_match_stays_sketched(cx: &mut TestAppContext) {
    let (dir, _window, _workspace, view) = on_the_map(cx);
    std::fs::write(dir.path().join("src/net/limit.rs"), "pub struct Limiter;\n").unwrap();
    std::fs::write(dir.path().join("src/limit.rs"), "pub struct Limiter;\n").unwrap();
    let folder = dir.path().join(".jig/map/sketches");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(
        folder.join("limits.toml"),
        "title = \"Limits\"\nlevel = \"site\"\n\n[[box]]\nid = \"limiter\"\nlabel = \"Limiter\"\nat = [0.0, 0.0]\nsize = [180.0, 64.0]\n",
    )
    .unwrap();
    read_again(cx, &view, &dir);
    assert_eq!(
        cx.update(|cx| view.read(cx).match_of("limits", "limiter")),
        Match::Unsure(2)
    );
}

#[gpui_kit::test]
fn make_task_asks_the_agent_and_the_task_links_back(cx: &mut TestAppContext) {
    let (dir, window, workspace, view) = on_the_map(cx);
    a_saved_sketch(&dir);
    read_again(cx, &view, &dir);
    draw(cx, window);
    click_on(cx, window, "map-sketch-make-task");
    let prompt = cx.update(|cx| workspace.read(cx).agent_prompt()).unwrap();
    assert!(
        prompt.contains("Turn my sketch “Caching” into a task"),
        "{prompt}"
    );
    assert!(prompt.contains("create_task"), "{prompt}");
    assert!(
        prompt.contains("Sketch: [Caching](.jig/map/sketches/caching.toml)"),
        "{prompt}"
    );
    assert!(
        prompt.contains("- box sketch:caching/cache"),
        "the sketch: {prompt}"
    );
    // The agent saves it, linking the sketch: the sketch's task.
    let task = jig_ai::tasks::create(
        dir.path(),
        "Add a cache",
        "Keep the last item.\n\nSketch: [Caching](.jig/map/sketches/caching.toml)",
        vec![jig_ai::tasks::Todo::new("Add `Cache`")],
    )
    .unwrap();
    view.update(cx, |view, cx| {
        view.link_tasks();
        cx.notify();
    });
    assert!(sketch_file(&dir, "caching").contains(&format!("task = \"{}\"", task.id)));
    // Implement runs that task.
    draw(cx, window);
    click_on(cx, window, "map-sketch-implement");
    let prompt = cx.update(|cx| workspace.read(cx).agent_prompt()).unwrap();
    assert!(
        prompt.contains("Implement the task “Add a cache”"),
        "{prompt}"
    );
    assert!(prompt.contains("- [ ] Add `Cache`"), "{prompt}");
    assert!(
        prompt.contains(".jig/map/sketches/caching.toml"),
        "{prompt}"
    );
}
