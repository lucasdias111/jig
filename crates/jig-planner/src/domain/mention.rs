use super::{Mentioned, Symbol, mention_at};

#[test]
fn finds_the_mention_before_the_cursor() {
    assert_eq!(mention_at("See @par", 8), Some((4, "par")));
    assert_eq!(mention_at("@", 1), Some((0, "")));
    assert_eq!(mention_at("(@Cache::get", 12), Some((1, "Cache::get")));
    assert_eq!(mention_at("See @par and", 12), None, "past a space");
    assert_eq!(mention_at("me@example.com", 14), None, "inside a word");
    assert_eq!(
        mention_at("See @par", 6),
        Some((4, "p")),
        "up to the cursor"
    );
}

#[test]
fn links_name_their_place() {
    let symbol = Mentioned::Symbol(Symbol {
        name: "run".into(),
        kind: "fn".into(),
        path: "src/a.rs".into(),
        line: 3,
    });
    assert_eq!(symbol.link(), "[`run`](src/a.rs#L3)");
    assert_eq!(
        Mentioned::File("src/a.rs".into()).link(),
        "[`a.rs`](src/a.rs)"
    );
}
