//! Moving through the Map's levels: from the code, by keys, zooming, the ruler and pinches.

use super::*;

#[gpui_kit::test]
fn show_map_flies_out_of_the_function_at_the_cursor(cx: &mut TestAppContext) {
    let (dir, window, workspace) = open_project(cx);
    press(cx, window, "secondary-b");
    let level = read_level(cx, &workspace);
    assert!(plan_of("save")(&level), "{level:?}");
    let view = map(cx, &workspace).unwrap();
    let (titles, selected, fading) = cx.update(|cx| {
        let view = view.read(cx);
        let titles: Vec<String> = view.plan().nodes.iter().map(|n| n.title.clone()).collect();
        (titles, view.selected_keys().to_vec(), view.crossfading())
    });
    assert_eq!(titles, ["save", "check", "write", "main"]);
    assert_eq!(selected.len(), 1, "you are here: the function came from");
    assert!(selected[0].contains("save"));
    assert!(fading, "flown out, fading in");
    // Its signature on its box.
    let signature = cx.update(|cx| view.read(cx).plan().nodes[0].detail.clone());
    assert_eq!(signature, "(item: u8) → bool");
    // ⌘B again: the code.
    press(cx, window, "secondary-b");
    assert!(map(cx, &workspace).is_none());
    // The places are kept, out of git.
    assert!(dir.path().join(".jig/map/places.json").is_file());
    let ignore = std::fs::read_to_string(dir.path().join(".jig/.gitignore")).unwrap();
    assert!(ignore.lines().any(|line| line == "map/places.json"));
}

#[gpui_kit::test]
fn the_title_bar_switch_shows_the_map_and_the_code(cx: &mut TestAppContext) {
    let (_dir, window, workspace) = open_project(cx);
    draw(cx, window);
    let mut vcx = gpui_kit::VisualTestContext::from_window(window, cx);
    let button = vcx
        .debug_bounds("mode-map")
        .expect("the Map's segment")
        .center();
    vcx.simulate_click(button, Modifiers::none());
    read_level(cx, &workspace);
    draw(cx, window);
    let mut vcx = gpui_kit::VisualTestContext::from_window(window, cx);
    let button = vcx.debug_bounds("mode-code").unwrap().center();
    vcx.simulate_click(button, Modifiers::none());
    assert!(map(cx, &workspace).is_none());
}

#[gpui_kit::test]
fn keys_step_between_code_plan_and_site(cx: &mut TestAppContext) {
    let (_dir, window, workspace) = open_project(cx);
    press(cx, window, "secondary-b");
    read_level(cx, &workspace);
    let view = map(cx, &workspace).unwrap();
    // Out to Site: the file the function's in selected.
    press(cx, window, "secondary-up");
    assert_eq!(level(cx, &workspace), Some(Level::Site));
    let selected = cx.update(|cx| view.read(cx).selected_keys().to_vec());
    assert_eq!(selected, ["site:file:src/store.rs"]);
    assert!(cx.update(|cx| view.read(cx).crossfading()));
    // In again: the file's Plan.
    press(cx, window, "secondary-down");
    assert_eq!(
        level(cx, &workspace),
        Some(Level::Plan(Focus::Module("src/store.rs".into())))
    );
    // Enter on a function: its Plan; Enter on that: its code.
    let check = cx.update(|cx| view.read(cx).titled("check"));
    let key = cx.update(|cx| view.read(cx).plan().nodes[check].key.clone());
    draw(cx, window);
    let at = cx.update(|cx| view.read(cx).node_point(check));
    let mut vcx = gpui_kit::VisualTestContext::from_window(window, cx);
    vcx.simulate_click(at, Modifiers::none());
    assert_eq!(
        cx.update(|cx| view.read(cx).selected_keys().to_vec()),
        [key]
    );
    press(cx, window, "enter");
    assert!(plan_of("check")(&level(cx, &workspace).unwrap()));
    press(cx, window, "enter");
    assert!(map(cx, &workspace).is_none(), "into its code");
    let cursor = cx.update(|cx| workspace.read(cx).editor().cursor(cx));
    assert_eq!(cursor, STORE.find("check(item: u8)").unwrap());
    // Go Back retraces it all: the Plan, the file's, Site, then the code.
    press(cx, window, GO_BACK);
    assert!(plan_of("check")(&level(cx, &workspace).unwrap()));
    press(cx, window, GO_BACK);
    assert_eq!(
        level(cx, &workspace),
        Some(Level::Plan(Focus::Module("src/store.rs".into())))
    );
    press(cx, window, GO_BACK);
    assert_eq!(level(cx, &workspace), Some(Level::Site));
    press(cx, window, GO_BACK);
    assert!(plan_of("save")(&level(cx, &workspace).unwrap()));
    press(cx, window, GO_BACK);
    assert!(map(cx, &workspace).is_none(), "back in the code");
}

#[gpui_kit::test]
fn zooming_past_a_level_crosses_into_the_next(cx: &mut TestAppContext) {
    let (_dir, window, workspace) = open_project(cx);
    press(cx, window, "secondary-b");
    read_level(cx, &workspace);
    let view = map(cx, &workspace).unwrap();
    settle(cx, window, &view);
    // Within Plan, zooming only zooms.
    let middle = cx.update(|cx| view.read(cx).node_point(0));
    zoom_scroll(cx, window, middle, 60.);
    assert!(plan_of("save")(&level(cx, &workspace).unwrap()));
    assert!(cx.update(|cx| view.read(cx).zoom()) < 1.);
    // Past its far end, out to Site.
    for _ in 0..6 {
        zoom_scroll(cx, window, middle, 120.);
        if level(cx, &workspace) == Some(Level::Site) {
            break;
        }
    }
    assert_eq!(level(cx, &workspace), Some(Level::Site));
    // The gesture that crossed goes no further until it's let go.
    let zoom = cx.update(|cx| view.read(cx).zoom());
    zoom_scroll(cx, window, middle, 120.);
    assert_eq!(cx.update(|cx| view.read(cx).zoom()), zoom);
    settle(cx, window, &view);
    // Zooming in over a file: in place, then into its Plan.
    for _ in 0..30 {
        let at = cx.update(|cx| view.read(cx).site_point("file:src/store.rs"));
        zoom_scroll(cx, window, at, -150.);
        draw(cx, window);
        if level(cx, &workspace) != Some(Level::Site) {
            break;
        }
    }
    assert_eq!(
        level(cx, &workspace),
        Some(Level::Plan(Focus::Module("src/store.rs".into())))
    );
    settle(cx, window, &view);
    // Over a function, in to its Plan; over that one, in to its code.
    for target in ["save", ""] {
        for _ in 0..12 {
            let ix = cx.update(|cx| {
                let view = view.read(cx);
                match target {
                    "" => view.plan().focus.unwrap(),
                    title => view.titled(title),
                }
            });
            let at = cx.update(|cx| view.read(cx).node_point(ix));
            zoom_scroll(cx, window, at, -150.);
            if target == "save" && plan_of("save")(&level(cx, &workspace).unwrap_or(Level::Site)) {
                break;
            }
            if target.is_empty() && map(cx, &workspace).is_none() {
                break;
            }
        }
        if target == "save" {
            assert!(plan_of("save")(&level(cx, &workspace).unwrap()));
            settle(cx, window, &view);
        }
    }
    assert!(map(cx, &workspace).is_none(), "zoomed into its code");
}

#[gpui_kit::test]
fn command_plus_and_minus_zoom_the_map_in_steps(cx: &mut TestAppContext) {
    let (_dir, window, workspace) = open_project(cx);
    press(cx, window, "secondary-b");
    read_level(cx, &workspace);
    let view = map(cx, &workspace).unwrap();
    settle(cx, window, &view);
    let zoom = |cx: &mut TestAppContext| cx.update(|cx| view.read(cx).zoom());
    let start = zoom(cx);
    press(cx, window, "secondary-=");
    assert!(zoom(cx) > start * 1.2, "a step closer");
    assert!(plan_of("save")(&level(cx, &workspace).unwrap()));
    press(cx, window, "secondary--");
    assert!((zoom(cx) - start).abs() < 0.01, "and back");
    press(cx, window, "secondary--");
    assert!(zoom(cx) < start, "a step further out");
    press(cx, window, "secondary-0");
    assert!((zoom(cx) - start).abs() < 0.01, "the Plan as it starts");
    // ⌘+ as well, where + has its own key.
    press(cx, window, "secondary-+");
    assert!(zoom(cx) > start * 1.2, "⌘+ is closer too");
}

#[gpui_kit::test]
fn the_ruler_shows_how_far_out_and_goes_to_a_level(cx: &mut TestAppContext) {
    let (_dir, window, workspace) = open_project(cx);
    press(cx, window, "secondary-b");
    read_level(cx, &workspace);
    let view = map(cx, &workspace).unwrap();
    settle(cx, window, &view);
    let depth = |cx: &mut TestAppContext| cx.update(|cx| view.read(cx).depth());
    let start = depth(cx);
    assert!((0.5..1.5).contains(&start), "at Plan: {start}");
    // Zooming out moves the needle towards Site, before it gets there.
    press(cx, window, "secondary--");
    let out = depth(cx);
    assert!(out > start && out < 2., "{start} then {out}");
    assert!(plan_of("save")(&level(cx, &workspace).unwrap()));
    // Clicking a level's name goes there.
    draw(cx, window);
    let mut vcx = gpui_kit::VisualTestContext::from_window(window, cx);
    let site = vcx
        .debug_bounds("map-level-Site")
        .expect("the ruler's Site")
        .center();
    vcx.simulate_click(site, Modifiers::none());
    assert_eq!(level(cx, &workspace), Some(Level::Site));
    assert!((1.5..=2.5).contains(&depth(cx)));
    draw(cx, window);
    let mut vcx = gpui_kit::VisualTestContext::from_window(window, cx);
    let code = vcx.debug_bounds("map-level-Code").unwrap().center();
    vcx.simulate_click(code, Modifiers::none());
    assert!(matches!(level(cx, &workspace), Some(Level::Plan(_))));
    // At Site with nothing picked, Code goes back to where you were.
    press(cx, window, "secondary-up");
    assert_eq!(level(cx, &workspace), Some(Level::Site));
    press(cx, window, "escape");
    assert!(cx.update(|cx| view.read(cx).selected_keys().is_empty()));
    draw(cx, window);
    let mut vcx = gpui_kit::VisualTestContext::from_window(window, cx);
    let code = vcx.debug_bounds("map-level-Code").unwrap().center();
    vcx.simulate_click(code, Modifiers::none());
    assert!(map(cx, &workspace).is_none(), "back in the code");
    let cursor = cx.update(|cx| workspace.read(cx).editor().cursor(cx));
    assert_eq!(cursor, STORE.find("check(item);").unwrap());
}

#[gpui_kit::test]
fn a_pinch_out_in_the_code_shows_the_map(cx: &mut TestAppContext) {
    let (_dir, window, workspace) = open_project(cx);
    draw(cx, window);
    let mut vcx = gpui_kit::VisualTestContext::from_window(window, cx);
    vcx.simulate_mouse_move(point(px(400.), px(250.)), None, Modifiers::none());
    for _ in 0..4 {
        vcx.simulate_event(PinchEvent {
            position: point(px(400.), px(250.)),
            delta: -0.05,
            modifiers: Modifiers::none(),
            phase: TouchPhase::Moved,
        });
    }
    vcx.run_until_parked();
    let level = read_level(cx, &workspace);
    assert!(plan_of("save")(&level));
}

#[gpui_kit::test]
fn a_pinch_in_the_code_shows_the_ruler_on_its_way_out(cx: &mut TestAppContext) {
    let (_dir, window, workspace) = open_project(cx);
    draw(cx, window);
    let mut vcx = gpui_kit::VisualTestContext::from_window(window, cx);
    assert!(
        vcx.debug_bounds("map-ruler").is_none(),
        "not while just coding"
    );
    let at = point(px(400.), px(250.));
    vcx.simulate_mouse_move(at, None, Modifiers::none());
    let pinch = |phase, delta| PinchEvent {
        position: at,
        delta,
        modifiers: Modifiers::none(),
        phase,
    };
    vcx.simulate_event(pinch(TouchPhase::Moved, -0.05));
    vcx.run_until_parked();
    draw(cx, window);
    let mut vcx = gpui_kit::VisualTestContext::from_window(window, cx);
    assert!(vcx.debug_bounds("map-ruler").is_some(), "on its way out");
    assert!(map(cx, &workspace).is_none(), "not there yet");
    // Let go short of the Map: the ruler goes.
    let mut vcx = gpui_kit::VisualTestContext::from_window(window, cx);
    vcx.simulate_event(pinch(TouchPhase::Ended, 0.));
    vcx.run_until_parked();
    draw(cx, window);
    let mut vcx = gpui_kit::VisualTestContext::from_window(window, cx);
    assert!(vcx.debug_bounds("map-ruler").is_none());
}
