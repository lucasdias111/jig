//! Domains edited by hand, updated, copied, and their names and flows drawn.

use super::*;

#[gpui_kit::test]
fn domains_are_renamed_merged_and_undone_by_hand(cx: &mut TestAppContext) {
    let (dir, window, workspace) = open_project(cx, true);
    let view = to_domain(cx, window, &workspace);
    // Rename: select it, F2, type, Enter.
    let at = cx.update(|cx| view.read(cx).region_point("Bookings", false));
    click(cx, window, at);
    press(cx, window, "f2");
    assert!(cx.update(|cx| view.read(cx).domain_input()).is_some());
    press(cx, window, "secondary-a");
    type_slowly(cx, window, "Reservations");
    press(cx, window, "enter");
    let text = saved(&dir);
    assert!(text.contains("name = \"Reservations\""), "{text}");
    assert!(
        text.contains("from = \"Reservations\""),
        "the flow follows: {text}"
    );
    // Merge: Events dropped onto Reservations.
    draw(cx, window);
    let (from, to) = cx.update(|cx| {
        (
            view.read(cx).region_point("Events", false),
            view.read(cx).region_point("Reservations", true),
        )
    });
    drag(cx, window, from, to);
    let text = saved(&dir);
    assert_eq!(text.matches("[[domain]]").count(), 1, "{text}");
    assert!(text.contains("\"web/\""), "its members came along: {text}");
    // ⌘Z undoes the merge, then the rename.
    press(cx, window, "secondary-z");
    assert_eq!(saved(&dir).matches("[[domain]]").count(), 2);
    press(cx, window, "secondary-z");
    assert!(saved(&dir).contains("name = \"Bookings\""));
    press(cx, window, "secondary-shift-z");
    assert!(saved(&dir).contains("name = \"Reservations\""));
}

#[gpui_kit::test]
fn a_file_dragged_at_site_moves_to_the_domain_it_lands_on(cx: &mut TestAppContext) {
    let (dir, window, workspace) = open_project(cx, true);
    let view = to_domain(cx, window, &workspace);
    press(cx, window, "secondary-down");
    let view = map(cx, &workspace).unwrap_or(view);
    view.update(cx, |view, _| {
        view.end_gesture();
        view.show_site_box("file:src/util.rs");
    });
    draw(cx, window);
    assert_eq!(cx.update(|cx| view.read(cx).level().clone()), Level::Site);
    // Tinted by their domains, until the switch is off.
    assert_eq!(
        cx.update(|cx| view.read(cx).tint_at("src/events/mod.rs")),
        Some(1)
    );
    assert_eq!(cx.update(|cx| view.read(cx).tint_at("src/util.rs")), None);
    let (from, to) = cx.update(|cx| {
        (
            view.read(cx).site_point("file:src/util.rs"),
            view.read(cx).site_point("file:web/App.svelte"),
        )
    });
    drag(cx, window, from, to);
    let text = saved(&dir);
    assert!(
        text.contains("\"src/events/\",\n  \"web/\",\n  \"src/util.rs\""),
        "{text}"
    );
    assert!(!text.contains("members = [\n  \"src/util.rs\""));
    view.update(cx, |view, cx| view.set_tints(false, cx));
    assert_eq!(
        cx.update(|cx| view.read(cx).tint_at("src/events/mod.rs")),
        None
    );
}

#[gpui_kit::test]
fn update_domains_places_only_the_unsorted_files(cx: &mut TestAppContext) {
    let (dir, window, workspace) = open_project(cx, true);
    std::fs::create_dir_all(dir.path().join("src/gifts")).unwrap();
    std::fs::write(
        dir.path().join("src/gifts/mod.rs"),
        "pub fn give_gift() {}\n",
    )
    .unwrap();
    let view = to_domain(cx, window, &workspace);
    assert!(
        cx.update(|cx| view.read(cx).domain_regions())
            .contains(&"Unsorted".to_string())
    );
    click_on(cx, window, "map-update-domains");
    let prompt = cx.update(|cx| workspace.read(cx).agent_prompt()).unwrap();
    assert!(prompt.contains("- src/gifts/mod.rs\n"));
    assert!(
        prompt.contains("name = \"Bookings\""),
        "the saved domains come along"
    );
    answer(
        cx,
        window,
        &workspace,
        r#"```toml
[[place]]
file = "src/gifts/mod.rs"
domain = "Gift Cards"

[[place]]
file = "src/events/mod.rs"
domain = "Gift Cards"

[[new_domain]]
name = "Gift Cards"
purpose = "Give a gift."
```"#,
    );
    let view = map(cx, &workspace).unwrap_or(view);
    press(cx, window, "secondary-enter");
    let text = saved(&dir);
    assert!(text.contains("name = \"Gift Cards\""));
    assert!(
        text.contains("members = [\n  \"src/gifts/mod.rs\"\n]"),
        "{text}"
    );
    // What was confirmed stays where it was.
    assert!(
        text.contains("members = [\n  \"src/events/\",\n  \"web/\"\n]"),
        "{text}"
    );
    assert!(
        !cx.update(|cx| view.read(cx).domain_regions())
            .contains(&"Unsorted".to_string())
    );
}

#[gpui_kit::test]
fn the_domains_are_copied_as_markdown_and_drawn_as_an_image(cx: &mut TestAppContext) {
    let (_dir, window, workspace) = open_project(cx, true);
    let view = to_domain(cx, window, &workspace);
    click_on(cx, window, "map-copy-markdown");
    let copied = cx
        .read_from_clipboard()
        .and_then(|item| item.text())
        .expect("copied");
    assert!(
        copied.contains("## Bookings\n\nBook a place at an event.\n\nOffers:\n- Booking form\n")
    );
    assert!(copied.contains("- Bookings asks for events Events\n"));
    let svg = cx.update(|cx| view.read(cx).export_svg(cx));
    assert!(svg.starts_with("<svg"));
    assert!(svg.contains(">Bookings</text>") && svg.contains("asks for events"));
}

#[gpui_kit::test]
fn names_sit_above_their_regions_and_flows_in_the_gaps(cx: &mut TestAppContext) {
    let (_dir, window, workspace) = open_project(cx, true);
    to_domain(cx, window, &workspace);
    draw(cx, window);
    let mut vcx = gpui_kit::VisualTestContext::from_window(window, cx);
    let selector = |name: String| -> &'static str { Box::leak(name.into_boxed_str()) };
    let regions: Vec<gpui_kit::Bounds<gpui_kit::Pixels>> = (0..4)
        .filter_map(|ix| vcx.debug_bounds(selector(format!("map-region-{ix}"))))
        .collect();
    assert!(regions.len() >= 2, "{regions:?}");
    for (ix, region) in regions.iter().enumerate() {
        let name = vcx
            .debug_bounds(selector(format!("map-region-name-{ix}")))
            .expect("a name");
        // Above its region, whole.
        assert!(name.bottom() <= region.top() + px(0.5), "{name:?}");
        assert!(name.size.width > px(0.));
    }
    // The flow's label covers no region.
    let flow = vcx.debug_bounds("map-flow-0").expect("a flow label");
    for region in &regions {
        assert!(!flow.intersects(region), "{flow:?} covers {region:?}");
    }
}

#[gpui_kit::test]
fn flows_are_faint_but_for_the_selected_domains(cx: &mut TestAppContext) {
    let (_dir, window, workspace) = open_project(cx, true);
    let view = to_domain(cx, window, &workspace);
    // main.rs is Shared's: the Bookings–Events flow isn't its, so faint.
    let selected = cx.update(|cx| view.read(cx).selected_keys().to_vec());
    assert_eq!(selected, ["domain:/shared"]);
    assert_eq!(cx.update(|cx| view.read(cx).drawn_faint()), [true]);
    // Bookings picked: its flow in full.
    let at = cx.update(|cx| view.read(cx).region_point("Bookings", true));
    click(cx, window, at);
    draw(cx, window);
    assert_eq!(cx.update(|cx| view.read(cx).drawn_faint()), [false]);
    // Nothing picked: faint again.
    press(cx, window, "escape");
    draw(cx, window);
    assert_eq!(cx.update(|cx| view.read(cx).drawn_faint()), [true]);
}

#[gpui_kit::test]
fn flows_stay_put_as_the_view_pans(cx: &mut TestAppContext) {
    let (_dir, window, workspace) = open_project(cx, true);
    let view = to_domain(cx, window, &workspace);
    let at = cx.update(|cx| view.read(cx).region_point("Bookings", true));
    click(cx, window, at);
    draw(cx, window);
    let label = |cx: &mut TestAppContext| {
        let mut vcx = gpui_kit::VisualTestContext::from_window(window, cx);
        vcx.debug_bounds("map-flow-0").expect("a flow label").origin
    };
    let mut was = label(cx);
    // Each nudge of the view moves the label with it, no more, no less,
    // out past the window's edge too.
    let step = point(px(-40.), px(0.));
    let mut gone_past = false;
    for _ in 0..40 {
        let mut vcx = gpui_kit::VisualTestContext::from_window(window, cx);
        vcx.simulate_event(gpui_kit::ScrollWheelEvent {
            position: at,
            delta: gpui_kit::ScrollDelta::Pixels(step),
            modifiers: Modifiers::none(),
            touch_phase: gpui_kit::TouchPhase::Moved,
        });
        vcx.run_until_parked();
        draw(cx, window);
        let now = label(cx);
        assert_eq!(now - was, step, "it moved with the view");
        was = now;
        if now.x < px(0.) {
            gone_past = true;
            break;
        }
    }
    assert!(gone_past, "it went off the edge with its link");
}
