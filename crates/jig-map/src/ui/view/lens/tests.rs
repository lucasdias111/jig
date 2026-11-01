use super::{Landing, fold};
use crate::domain::layout::Rect;

fn landing(key: &str) -> Option<Landing> {
    Some(Landing {
        key: key.into(),
        rect: Rect::default(),
    })
}

#[test]
fn steps_in_a_row_on_one_box_fold_into_one_stop() {
    let stops = fold(&[landing("a"), landing("a"), None, landing("b"), landing("a")]);
    let ranges: Vec<(String, String)> = stops
        .iter()
        .map(|stop| (stop.landing.key.clone(), stop.range()))
        .collect();
    assert_eq!(
        ranges,
        [
            ("a".into(), "1–2".into()),
            ("b".into(), "4".into()),
            ("a".into(), "5".into())
        ]
    );
}
