use super::{clip, escape, png, wrap};

#[test]
fn long_words_wrap_and_markup_is_escaped() {
    assert_eq!(
        wrap("Book a class and pay for it online", 120., 12.5),
        ["Book a class and", "pay for it online"]
    );
    assert_eq!(escape("<a & b>"), "&lt;a &amp; b&gt;");
    assert_eq!(clip("abcdefghijklmnop", 40., 10.), "abcdef…");
}

#[test]
fn an_svg_becomes_a_png() {
    let bytes = png(r##"<svg xmlns="http://www.w3.org/2000/svg" width="20" height="10"><rect width="20" height="10" fill="#336699"/></svg>"##).unwrap();
    assert_eq!(&bytes[1..4], b"PNG");
}
