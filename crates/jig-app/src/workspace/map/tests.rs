use serde_json::json;

use super::{Texts, package_of, targets};
use jig_lsp::lsp;
use jig_map::{Project, Target};

#[test]
fn a_call_hierarchy_answer_maps_onto_the_project() {
    let dir = jig_map::fixture_project();
    let project = Project::read(dir.path());
    let users = dir
        .path()
        .join("server/src/org/demo/users/UserService.java");
    let answer = json!([
        {"to": {
            "name": "find", "kind": 6,
            "uri": lsp::file_uri(&users),
            "range": {"start": {"line": 3, "character": 4}, "end": {"line": 5, "character": 5}},
            "selectionRange": {"start": {"line": 3, "character": 16}, "end": {"line": 3, "character": 20}}
        }, "fromRanges": []},
        {"to": {
            "name": "info", "kind": 6, "detail": "org.slf4j.Logger",
            "uri": "jdt://contents/slf4j-api.jar/org.slf4j/Logger.class",
            "range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 0}},
            "selectionRange": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 0}}
        }, "fromRanges": []}
    ]);
    let found = targets(
        &project,
        &answer,
        "to",
        lsp::Encoding::Utf16,
        &mut Texts::default(),
    );
    let find = (0..project.functions.len())
        .find(|&id| project.functions[id].title() == "UserService.find")
        .unwrap();
    assert_eq!(
        found,
        [Target::Function(find), Target::External("org.slf4j".into())]
    );
}

#[test]
fn packages_outside_are_named_from_the_detail_or_node_modules() {
    assert_eq!(
        package_of(&json!({"detail": "org.springframework.http.ResponseEntity"})),
        "org.springframework.http"
    );
    assert_eq!(
        package_of(&json!({"uri": "file:///p/node_modules/@scope/lib/index.d.ts"})),
        "@scope/lib"
    );
}
