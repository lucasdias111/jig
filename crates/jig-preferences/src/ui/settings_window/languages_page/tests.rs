use super::names;

#[test]
fn names_languages_in_a_sentence() {
    assert_eq!(names(&["rust".into()]), "Rust");
    assert_eq!(
        names(&["typescript".into(), "tsx".into(), "javascript".into()]),
        "TypeScript, TSX and JavaScript"
    );
    assert_eq!(names(&["cobol".into()]), "cobol");
}
