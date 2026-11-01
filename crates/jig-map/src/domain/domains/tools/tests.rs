use super::*;
use crate::domain::index::tests::fixture;

const BUGS: &str = "server/src/org/demo/bugs/";
const USERS: &str = "server/src/org/demo/users/UserService.java";

fn files() -> Vec<String> {
    [
        "client/src/net/socket.ts",
        "client/src/rooms.ts",
        "server/src/org/demo/bugs/BugService.java",
        "server/src/org/demo/bugs/MailNotifier.java",
        "server/src/org/demo/bugs/Notifier.java",
        USERS,
        "web/App.svelte",
    ]
    .map(String::from)
    .to_vec()
}

fn index_call() -> Value {
    json!({
        "action": "index",
        "domains": [
            {"name": "Bug Reports", "purpose": "Report bugs.", "offers": ["Bug form"], "why": "Bugs.", "members": [BUGS, USERS]},
            {"name": "Rooms", "purpose": "Join rooms.", "offers": ["Room page"], "why": "Rooms.", "members": ["client/src/", "web/App.svelte"]}
        ],
        "flows": [{"from": "Rooms", "to": "Bug Reports", "label": "reports bugs to"}],
        "shared": []
    })
}

#[test]
fn a_call_proposes_what_the_same_toml_would() {
    let call = Action::Index
        .proposal_from_call(&index_call(), None, &files())
        .unwrap();
    let reply = format!("```toml\n{}```", call.to_toml());
    let toml = Action::Index.proposal(&reply, None).unwrap();
    assert_eq!(call, toml);
    assert_eq!(call.domains[0].offers, ["Bug form"]);
    assert_eq!(call.flows[0].label, "reports bugs to");
}

#[test]
fn unknown_files_and_names_and_unplaced_files_come_back_to_fix() {
    let mut call = index_call();
    call["domains"][0]["members"] = json!([BUGS, "server/UserService.java"]);
    call["flows"][0]["to"] = json!("Bugs");
    let error = Action::Index
        .proposal_from_call(&call, None, &files())
        .unwrap_err();
    assert!(
        error.contains(
            "server/UserService.java (did you mean server/src/org/demo/users/UserService.java)"
        ),
        "{error}"
    );
    assert!(error.contains("name no domain: Bugs"), "{error}");
    assert!(
        error.contains("these have none: server/src/org/demo/users/UserService.java"),
        "{error}"
    );
    let mut call = index_call();
    call["domains"][1]["name"] = json!("Shared");
    assert!(
        Action::Index
            .proposal_from_call(&call, None, &files())
            .unwrap_err()
            .contains("can't be a domain's name")
    );
    assert!(
        Action::Index
            .proposal_from_call(&json!({"action": "index", "domains": []}), None, &files())
            .is_err()
    );
}

/// Domains saved before `web/App.svelte` appeared.
fn saved() -> Domains {
    let mut call = index_call();
    call["domains"][1]["members"] = json!(["client/src/"]);
    Action::Index
        .proposal_from_call(&call, None, &files()[..6])
        .unwrap()
}

#[test]
fn an_update_places_every_unsorted_file_and_only_those() {
    let saved = saved();
    let action = action_of(&json!({"action": "update"}), Some(&saved), &files()).unwrap();
    assert_eq!(action, Action::Update(vec!["web/App.svelte".into()]));
    let call = json!({"action": "update", "placements": [{"file": "web/App.svelte", "domain": "Pages"}],
            "new_domains": [{"name": "Pages", "purpose": "Pages.", "offers": [], "why": "Pages.", "members": []}],
            "new_flows": [{"from": "Pages", "to": "Rooms", "label": "opens"}]});
    let proposal = action
        .proposal_from_call(&call, Some(&saved), &files())
        .unwrap();
    // The same as the TOML route makes of it.
    let toml = action
            .proposal(
                "```toml\n[[place]]\nfile = \"web/App.svelte\"\ndomain = \"Pages\"\n\n[[new_domain]]\nname = \"Pages\"\npurpose = \"Pages.\"\nwhy = \"Pages.\"\n\n[[new_flow]]\nfrom = \"Pages\"\nto = \"Rooms\"\nlabel = \"opens\"\n```",
                Some(&saved),
            )
            .unwrap();
    assert_eq!(proposal, toml);
    assert_eq!(proposal.home_of("web/App.svelte"), Home::Domain(2));
    let stray = json!({"action": "update", "placements": [
        {"file": "client/src/rooms.ts", "domain": "Bug Reports"},
        {"file": "web/App.svelte", "domain": "Nowhere"}
    ]});
    let error = action
        .proposal_from_call(&stray, Some(&saved), &files())
        .unwrap_err();
    assert!(
        error.contains("aren't among them: client/src/rooms.ts"),
        "{error}"
    );
    assert!(error.contains("web/App.svelte → Nowhere"), "{error}");
    let none = json!({"action": "update", "placements": []});
    assert!(
        action
            .proposal_from_call(&none, Some(&saved), &files())
            .unwrap_err()
            .contains("these have none: web/App.svelte")
    );
}

#[test]
fn a_split_covers_the_domains_files() {
    let saved = saved();
    let action = action_of(
        &json!({"action": "split", "domain": "Bug Reports"}),
        Some(&saved),
        &files(),
    )
    .unwrap();
    let call = json!({"action": "split", "domain": "Bug Reports", "domains": [
            {"name": "Bugs", "purpose": "", "offers": [], "why": "", "members": [BUGS]},
            {"name": "People", "purpose": "", "offers": [], "why": "", "members": [USERS]}
        ], "flows": [{"from": "Bugs", "to": "People", "label": "asks about"}]});
    let proposal = action
        .proposal_from_call(&call, Some(&saved), &files())
        .unwrap();
    let names: Vec<&str> = proposal.domains.iter().map(|d| d.name.as_str()).collect();
    assert_eq!(names, ["Bugs", "People", "Rooms"]);
    let mut short = call.clone();
    short["domains"][1]["members"] = json!([]);
    assert!(
        action
            .proposal_from_call(&short, Some(&saved), &files())
            .unwrap_err()
            .contains(USERS)
    );
    let mut taken = call;
    taken["domains"][1]["name"] = json!("Rooms");
    assert!(
        action
            .proposal_from_call(&taken, Some(&saved), &files())
            .unwrap_err()
            .contains("\"Rooms\" names more than one domain")
    );
    assert!(
        action_of(
            &json!({"action": "split", "domain": "Nope"}),
            Some(&saved),
            &files()
        )
        .is_err()
    );
}

#[test]
fn the_outline_lists_every_file_and_describes_a_folders() {
    let dir = fixture();
    std::fs::write(dir.path().join("client/src/App.svelte"), "<p>hi</p>").unwrap();
    let project = Project::read(dir.path());
    let overview = project_outline(&project, None, &json!({})).unwrap();
    assert!(overview.contains("- client/src/App.svelte\n"), "{overview}");
    assert!(overview.contains("calls between groups"), "{overview}");
    let detail = project_outline(
        &project,
        None,
        &json!({"folder": "server/src/org/demo/bugs"}),
    )
    .unwrap();
    assert!(
        detail.contains("### server/src/org/demo/bugs/BugService.java"),
        "{detail}"
    );
    assert!(
        detail.contains("BugService.create(title: String, userId: long) → Bug"),
        "{detail}"
    );
    assert!(
        detail.contains("imports: org.demo.users.UserService"),
        "{detail}"
    );
    assert!(
        detail.contains("server/src/org/demo/users/UserService.java (1)"),
        "{detail}"
    );
    let group = project_outline(&project, None, &json!({"group": 1})).unwrap();
    assert!(
        group.contains("### client/src/App.svelte (group 1)\nNot read"),
        "{group}"
    );
    let paths = project_outline(&project, None, &json!({"files_only": true})).unwrap();
    assert_eq!(paths.lines().count(), source_files(&project).len());
    assert!(project_outline(&project, None, &json!({"group": 99})).is_err());
    assert!(project_outline(&project, None, &json!({"folder": "nowhere/"})).is_err());
    assert!(project_outline(&project, None, &json!({"files_only": true, "page": 3})).is_err());
}
