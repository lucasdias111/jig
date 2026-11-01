use serde_json::json;

use super::*;
use crate::domain::index::tests::fixture;
use crate::store::cache::Folder;

#[test]
fn calls_become_drawings() {
    let trace = call(
        "map_trace",
        &json!({"title": "Signing in", "steps": [
            {"node": "a.rs#login", "label": "Check"}, {"place": "b.rs", "label": "Save"}
        ]}),
    )
    .unwrap();
    let Call::Draw(trace) = trace else {
        panic!("a drawing")
    };
    assert_eq!(trace.kind, Kind::Trace);
    assert_eq!(trace.places(), ["a.rs#login", "b.rs"]);
    assert_eq!(trace.summary(), "Trace: Signing in · 2 steps");
    assert_eq!(call("map_clear", &json!({})).unwrap(), Call::Clear);
    let note = call("map_note", &json!({"place": "a.rs", "text": "Slow"})).unwrap();
    assert!(
        matches!(note, Call::Draw(Drawing { kind: Kind::Note, ref title, .. }) if title == "Slow")
    );
    assert!(call("map_highlight", &json!({"title": "x", "places": []})).is_err());
    let Call::Draw(proposed) = call(
        "map_propose",
        &json!({"title": "Cache", "boxes": [{"name": "Cache", "near": "a.rs", "purpose": "Keeps"}],
                "arrows": [{"from": "Cache", "to": "b.rs"}]}),
    )
    .unwrap() else {
        panic!("a drawing")
    };
    assert_eq!(proposed.places(), ["b.rs", "a.rs"], "ghosts aren't places");
}

#[test]
fn places_resolve_against_the_index_with_suggestions() {
    let dir = fixture();
    let project = Project::read(dir.path());
    let users = "server/src/org/demo/users/UserService.java";
    assert_eq!(
        resolve(
            &project,
            None,
            &Folder(dir.path()),
            &format!("{users}#UserService.find")
        ),
        Ok(Place::File {
            path: users.into(),
            owner: Some("UserService".into()),
            function: Some("UserService.find".into())
        })
    );
    assert_eq!(
        resolve(
            &project,
            None,
            &Folder(dir.path()),
            &format!("./{users}#find")
        ),
        resolve(
            &project,
            None,
            &Folder(dir.path()),
            &format!("{users}#UserService.find")
        ),
        "by its short name too"
    );
    assert!(matches!(
        resolve(
            &project,
            None,
            &Folder(dir.path()),
            &format!("{users}#UserService")
        ),
        Ok(Place::File {
            owner: Some(_),
            function: None,
            ..
        })
    ));
    assert_eq!(
        resolve(&project, None, &Folder(dir.path()), "server/src/org/demo/"),
        Ok(Place::Folder("server/src/org/demo/".into()))
    );
    assert_eq!(
        resolve(&project, None, &Folder(dir.path()), "server/src/org/demo"),
        Ok(Place::Folder("server/src/org/demo/".into())),
        "without its slash"
    );
    let wrong = resolve(
        &project,
        None,
        &Folder(dir.path()),
        &format!("{users}#UserService.remove"),
    )
    .unwrap_err();
    assert!(
        wrong.contains(&format!("{users}#UserService.find")),
        "{wrong:?}"
    );
    let moved = resolve(
        &project,
        None,
        &Folder(dir.path()),
        "src/UserService.java#find",
    )
    .unwrap_err();
    assert!(
        moved.iter().any(|like| like.starts_with(users)),
        "{moved:?}"
    );
}

#[test]
fn a_drawing_is_checked_whole() {
    let dir = fixture();
    let project = Project::read(dir.path());
    let Call::Draw(drawing) = call(
        "map_highlight",
        &json!({"title": "x", "places": ["server/src/org/demo/", "nowhere.rs"]}),
    )
    .unwrap() else {
        panic!()
    };
    let error = check(&project, None, &Folder(dir.path()), &drawing).unwrap_err();
    assert!(error.contains("nowhere.rs"), "{error}");
    assert!(!error.contains("demo/ ("), "{error}");
}
