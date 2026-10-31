use super::line_start;

#[test]
fn line_starts() {
    let text = "a\nbc\n\nd";
    assert_eq!(line_start(text, 0), 0);
    assert_eq!(line_start(text, 1), 2);
    assert_eq!(line_start(text, 2), 5);
    assert_eq!(line_start(text, 3), 6);
    assert_eq!(line_start(text, 4), text.len());
}
