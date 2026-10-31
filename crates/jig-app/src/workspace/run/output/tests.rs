use gpui_kit::{HighlightStyle, Hsla};

use super::{OutputSelection, layer, word_at};
use jig_run::run_output::OutputLine;

#[test]
fn links_layer_over_colours() {
    let red = HighlightStyle {
        color: Some(Hsla::red()),
        ..Default::default()
    };
    let link = HighlightStyle {
        color: Some(Hsla::blue()),
        ..Default::default()
    };
    let layered = layer(&[(0..10, red)], &[(4..14, link)]);
    assert_eq!(layered, [(0..4, red), (4..10, link), (10..14, link)]);
}

#[test]
fn selection_copies_across_lines_either_way() {
    let lines = ["first line", "second", "third line"].map(OutputLine::meta);
    let down = OutputSelection {
        anchor: (0, 6),
        head: (2, 5),
    };
    assert_eq!(down.text(&lines), "line\nsecond\nthird");
    let up = OutputSelection {
        anchor: down.head,
        head: down.anchor,
    };
    assert_eq!(up.text(&lines), "line\nsecond\nthird");
    assert_eq!(down.range_in(1, 6), Some(0..6));
    assert_eq!(down.range_in(3, 4), None);
}

#[test]
fn selection_follows_dropped_lines() {
    let selection = OutputSelection {
        anchor: (5, 1),
        head: (7, 2),
    };
    assert_eq!(
        selection.shifted_up(5),
        Some(OutputSelection {
            anchor: (0, 1),
            head: (2, 2),
        })
    );
    assert_eq!(selection.shifted_up(6), None);
}

#[test]
fn double_click_takes_the_word() {
    let text = "error: cannot_find foo";
    assert_eq!(&text[word_at(text, 10)], "cannot_find");
    assert_eq!(&text[word_at(text, 5)], ":");
    assert_eq!(&text[word_at(text, text.len())], "foo");
}
