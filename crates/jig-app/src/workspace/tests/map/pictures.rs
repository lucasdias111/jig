//! Drawing real projects' Maps, to check them without a screen.

use super::*;

/// The Map on Jig's own repo, drawn at every level: `cargo test -p
/// jig-app map_of_jig_live -- --ignored --nocapture`.
#[gpui_kit::test]
#[ignore]
fn map_of_jig_live(cx: &mut TestAppContext) {
    let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let repo = super::super::super::tabs::canonical(&repo);
    let file = repo.join("crates/jig-app/src/workspace.rs");
    let text = std::fs::read_to_string(&file).unwrap();
    let (window, workspace) = open(cx, &file);
    let inside = text.find("fn render_code").unwrap() + 3;
    cx.update(|cx| {
        workspace
            .read(cx)
            .editor()
            .clone()
            .select(inside..inside, cx)
    });
    press(cx, window, "secondary-b");
    let arrived = read_level(cx, &workspace);
    assert!(plan_of("render_code")(&arrived), "{arrived:?}");
    let view = map(cx, &workspace).unwrap();
    draw(cx, window);
    let titles: Vec<String> = cx.update(|cx| {
        view.read(cx)
            .plan()
            .nodes
            .iter()
            .map(|n| n.title.clone())
            .collect()
    });
    println!("Plan of render_code: {titles:?}");
    assert!(
        titles.iter().any(|t| t == "Workspace.render"),
        "called from render"
    );
    press(cx, window, "secondary-up");
    settle(cx, window, &view);
    let selected = cx.update(|cx| view.read(cx).selected_keys().to_vec());
    // Its type's box, or its file's when that's the type's.
    assert!(
        selected.len() == 1 && selected[0].contains(":crates/jig-app/src/workspace.rs"),
        "{selected:?}"
    );
    let showing = cx.update(|cx| view.read(cx).site_showing());
    println!("Site around workspace.rs: {} boxes showing", showing.len());
    press(cx, window, "secondary-up");
    settle(cx, window, &view);
    println!(
        "The domains: {:?}",
        cx.update(|cx| view.read(cx).domain_regions())
    );
    let middle = point(px(400.), px(240.));
    for _ in 0..12 {
        zoom_scroll(cx, window, middle, -120.);
        draw(cx, window);
        if level(cx, &workspace) != Some(Level::Site) {
            break;
        }
    }
    let ended = level(cx, &workspace);
    println!("Zoomed in from the middle: {ended:?}");
}

/// Pictures of a project's Map, to look at the design without a screen,
/// written to `JIG_PICTURES`: its Plan, Site and Domain at stepped zooms,
/// then a zoom from Domain into the region holding the function, on
/// through Site to its Plan, in a window 1600 by 1000. `JIG_MAP_ROOT=<project> JIG_MAP_FILE=<file>
/// JIG_MAP_AT=<text in a function> JIG_PICTURES=<dir> cargo test -p jig-app
/// map_pictures_live -- --ignored --nocapture`.
#[gpui_kit::test]
#[ignore]
fn map_pictures_live(cx: &mut TestAppContext) {
    let file = std::path::PathBuf::from(std::env::var("JIG_MAP_FILE").unwrap());
    let at = std::env::var("JIG_MAP_AT").unwrap();
    let out = std::path::PathBuf::from(std::env::var("JIG_PICTURES").unwrap());
    std::fs::create_dir_all(&out).unwrap();
    let text = std::fs::read_to_string(&file).unwrap();
    let (window, workspace) = match std::env::var_os("JIG_MAP_ROOT") {
        Some(root) => {
            let (window, workspace) = open(cx, std::path::Path::new(&root));
            open_file(cx, window, &workspace, &file);
            (window, workspace)
        }
        None => open(cx, &file),
    };
    // A laptop's window, not the tests' small one.
    cx.simulate_window_resize(window, gpui_kit::size(px(1600.), px(1000.)));
    draw(cx, window);
    let inside = text.find(&at).unwrap() + 1;
    cx.update(|cx| {
        workspace
            .read(cx)
            .editor()
            .clone()
            .select(inside..inside, cx)
    });
    press(cx, window, "secondary-b");
    println!("arrived at {:?}", read_level(cx, &workspace));
    let view = map(cx, &workspace).unwrap();
    // The agent's drawings over every level, from `JIG_MAP_DRAWING`: a
    // JSON list of `{"tool": "map_trace", "input": {…}}`.
    if let Some(file) = std::env::var_os("JIG_MAP_DRAWING") {
        let calls: Vec<serde_json::Value> =
            serde_json::from_str(&std::fs::read_to_string(file).unwrap()).unwrap();
        let drawings = calls
            .iter()
            .filter_map(|call| {
                match jig_map::drawing_call(call["tool"].as_str()?, &call["input"]).ok()? {
                    jig_map::Call::Draw(drawing) => Some(drawing),
                    jig_map::Call::Clear => None,
                }
            })
            .enumerate()
            .collect();
        view.update(cx, |view, cx| view.set_drawings(drawings, cx));
        // `JIG_MAP_TOUR=<step>`: touring the first drawing, there.
        if let Some(step) = std::env::var("JIG_MAP_TOUR")
            .ok()
            .and_then(|step| step.parse().ok())
        {
            view.update(cx, |view, _| view.show_tour_step(0, step));
        }
    }
    draw(cx, window);
    let write = |name: String, bytes: Vec<u8>| {
        let path = out.join(format!("{name}.png"));
        std::fs::write(&path, bytes).unwrap();
        println!("wrote {}", path.display());
    };
    let save = |level: &str, view: &Entity<MapView>, cx: &mut TestAppContext| {
        for (name, bytes) in view.update(cx, |view, cx| view.pictures(cx)) {
            write(format!("{level}-{name}"), bytes);
        }
    };
    save("1-plan", &view, cx);
    press(cx, window, "secondary-up");
    settle(cx, window, &view);
    let selected = cx.update(|cx| view.read(cx).selected_keys().to_vec());
    println!("site selected: {selected:?}");
    let target = selected[0].trim_start_matches("site:").to_string();
    save("2-site", &view, cx);
    press(cx, window, "secondary-up");
    settle(cx, window, &view);
    save("3-domain", &view, cx);
    // The whole of Domain, then zooming over the function's file: Domain
    // and Site share their drawing, so it's where it'll be at Site.
    press(cx, window, "secondary-up");
    settle(cx, window, &view);
    if let Some(bytes) = view.update(cx, |view, cx| view.picture_now(cx)) {
        write("3-domain-canvas".into(), bytes);
    }
    for n in 0..60 {
        let at = cx.update(|cx| view.read(cx).site_point(&target));
        zoom_scroll(cx, window, at, -40.);
        view.update(cx, |view, _| view.end_gesture());
        draw(cx, window);
        if n % 3 == 2 {
            let now = level(cx, &workspace);
            println!("step {n}: {now:?}");
            if let Some(bytes) = view.update(cx, |view, cx| view.picture_now(cx)) {
                write(format!("4-zoom-{:02}", n / 3), bytes);
            }
            if !matches!(now, Some(Level::Domain) | Some(Level::Site)) {
                break;
            }
        }
    }
}
