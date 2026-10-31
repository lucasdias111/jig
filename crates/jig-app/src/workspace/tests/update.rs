//! A downloaded update in the status bar.

use super::*;
use jig_updates::{self as update, State};

#[gpui_kit::test]
fn a_downloaded_update_offers_a_restart(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("lib.rs");
    std::fs::write(&path, ORIGINAL).unwrap();
    let (window, _workspace) = open(cx, &path);
    let mut vcx = gpui_kit::VisualTestContext::from_window(window, cx);
    let shown = |vcx: &mut gpui_kit::VisualTestContext| {
        vcx.update(|window, cx| window.render_frame(cx));
        vcx.debug_bounds("update")
    };
    assert!(shown(&mut vcx).is_none(), "nothing until one is found");

    let version = update::Version(9, 0, 0);
    vcx.update(|_, cx| {
        update::set_for_test(
            State::Ready {
                version,
                archive: dir.path().join("Jig.zip"),
            },
            cx,
        )
    });
    let bounds = shown(&mut vcx).expect("Restart to Update");

    // Tests run from cargo's target folder, which Jig won't replace: the
    // restart says why and offers the download instead.
    vcx.simulate_click(bounds.center(), gpui_kit::Modifiers::none());
    vcx.run_until_parked();
    let state = vcx.update(|_, cx| update::state(cx));
    assert!(
        matches!(&state, State::Manual { reason, .. } if reason.contains("source")),
        "{state:?}"
    );
    assert!(shown(&mut vcx).is_some(), "now a link to the download");
}
