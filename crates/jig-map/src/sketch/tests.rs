use super::*;
use crate::index::tests::fixture;

fn sketch() -> Sketch {
    Sketch {
        name: "rate-limiting".into(),
        title: "Rate limiting".into(),
        level: "site".into(),
        task: None,
        archived: false,
        boxes: vec![SketchBox {
            id: "rate-limiter".into(),
            label: "RateLimiter".into(),
            purpose: "Keeps each user's budget".into(),
            near: "src/api/".into(),
            at: [40., 120.],
            size: [180., 64.],
        }],
        arrows: vec![SketchArrow {
            from: "src/api/handlers.rs".into(),
            to: "rate-limiter".into(),
            label: "checks".into(),
        }],
        notes: vec![Note {
            id: "note".into(),
            text: "Per user, not per address".into(),
            on: "rate-limiter".into(),
            near: String::new(),
            at: [10., -40.],
            size: [160., 70.],
        }],
        frames: Vec::new(),
    }
}

#[test]
fn sketches_are_saved_as_readable_toml_and_read_back() {
    let root = tempfile::tempdir().unwrap();
    let sketch = sketch();
    save(root.path(), &sketch).unwrap();
    let text = std::fs::read_to_string(file(root.path(), "rate-limiting")).unwrap();
    assert!(text.contains("level = \"site\""), "{text}");
    assert!(text.contains("[[box]]"), "{text}");
    assert!(text.contains("near = \"src/api/\""), "{text}");
    assert!(text.contains("to = \"rate-limiter\""), "{text}");
    assert!(!text.contains("archived"), "{text}");
    assert_eq!(read_all(root.path()), std::slice::from_ref(&sketch));
    assert_eq!(sketch.attached(), ["src/api/handlers.rs"]);
    assert_eq!(sketch.places(), ["src/api/handlers.rs", "src/api/"]);
    remove(root.path(), "rate-limiting").unwrap();
    assert!(read_all(root.path()).is_empty());
}

#[test]
fn names_and_keys_go_both_ways() {
    let shape = ShapeRef {
        sketch: "limits".into(),
        kind: ShapeKind::Note,
        id: "note-2".into(),
    };
    assert_eq!(ShapeRef::of_key(&shape.key()), Some(shape));
    assert_eq!(
        parse_place("sketch:limits/rate-limiter"),
        Some(("limits".into(), "rate-limiter".into()))
    );
    assert_eq!(of_link(&link("limits")), Some("limits".into()));
    assert_eq!(of_link("src/a.toml"), None);
    assert_eq!(
        linked_in(
            "Sketch: [Limits](.jig/map/sketches/limits.toml) and [x](.jig/map/sketches/x.toml)"
        ),
        ["limits", "x"]
    );
    let taken = vec!["box".to_string(), "box-2".to_string()];
    assert_eq!(unique("box", &taken), "box-3");
    assert_eq!(slug("Rate limiting!", "x"), "rate-limiting");
}

#[test]
fn a_box_is_matched_only_when_its_sure() {
    let dir = fixture();
    let project = Project::read(dir.path());
    let users = "server/src/org/demo/users/UserService.java";
    assert_eq!(
        match_box(&project, "User service", None),
        Match::Real(format!("{users}#UserService"))
    );
    assert_eq!(match_box(&project, "Nothing like it", None), Match::None);
    assert_eq!(match_box(&project, "ab", None), Match::None, "too short");
}

#[test]
fn several_of_one_name_are_narrowed_by_where_the_box_is() {
    let dir = tempfile::tempdir().unwrap();
    let write = |name: &str, text: &str| {
        let path = dir.path().join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    };
    write("a/limit.rs", "pub struct Limiter;\n");
    write("b/limit.rs", "pub struct Limiter;\n");
    write("b/c/limit.rs", "pub struct Limiter;\n");
    let project = Project::read(dir.path());
    assert_eq!(match_box(&project, "Limiter", None), Match::Unsure(3));
    assert_eq!(
        match_box(&project, "Limiter", Some("b/")),
        Match::Real("b/limit.rs#Limiter".into()),
        "its own folder's over one deeper"
    );
    assert_eq!(
        match_box(&project, "Limiter", Some("b/c/")),
        Match::Real("b/c/limit.rs#Limiter".into())
    );
}
