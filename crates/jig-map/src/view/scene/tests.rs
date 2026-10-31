use super::*;

#[test]
fn arrows_leave_from_the_edges() {
    let a = Rect {
        x: 0.,
        y: 0.,
        width: 100.,
        height: 50.,
    };
    let b = Rect { x: 300., ..a };
    assert_eq!(edge_points(a, b), ((100., 25.), (300., 25.)));
}
