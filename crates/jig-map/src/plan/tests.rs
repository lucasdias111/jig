use super::super::index::tests::{fixture, id};
use super::*;

fn titles(plan: &Plan) -> Vec<&str> {
    plan.nodes.iter().map(|n| n.title.as_str()).collect()
}

fn wired(plan: &Plan, from: &str, to: &str) -> Option<WireKind> {
    let from = plan.nodes.iter().position(|n| n.title == from)?;
    let to = plan.nodes.iter().position(|n| n.title == to)?;
    plan.wires
        .iter()
        .find(|w| w.from == from && w.to == to)
        .map(|w| w.kind.clone())
}

#[test]
fn a_function_plan_is_one_hop_with_externals_and_interfaces() {
    let dir = fixture();
    let project = Project::read(dir.path());
    let create = id(&project, "BugService.create");
    let plan = neighbourhood(&project, create, &Opened::default());
    assert_eq!(
        titles(&plan),
        [
            "BugService.create",
            "UserService.find",
            "BugService.save",
            "Notifier.send",
            "org.slf4j"
        ]
    );
    assert_eq!(
        wired(&plan, "BugService.create", "UserService.find"),
        Some(WireKind::Call)
    );
    assert_eq!(
        wired(&plan, "BugService.create", "org.slf4j"),
        Some(WireKind::Call)
    );
    let focus = &plan.nodes[plan.focus.unwrap()];
    assert_eq!(focus.detail, "(title: String, userId: long) → Bug");
    let send = plan
        .nodes
        .iter()
        .find(|n| n.title == "Notifier.send")
        .unwrap();
    assert_eq!(send.implementations, 1, "one implementation to open");
    let find = plan
        .nodes
        .iter()
        .find(|n| n.title == "UserService.find")
        .unwrap();
    assert_eq!(find.hidden_callees, 1, "lookup, a hop further");
    let frames: Vec<&str> = plan.frames.iter().map(|f| f.title.as_str()).collect();
    assert_eq!(
        frames,
        [
            "class BugService",
            "class UserService",
            "interface Notifier"
        ]
    );
}

#[test]
fn opening_a_box_adds_its_next_hop() {
    let dir = fixture();
    let project = Project::read(dir.path());
    let create = id(&project, "BugService.create");
    let mut opened = Opened::default();
    opened
        .callees
        .insert(function_key(&project, id(&project, "UserService.find")));
    opened
        .implementations
        .insert(function_key(&project, id(&project, "Notifier.send")));
    let plan = neighbourhood(&project, create, &opened);
    assert!(titles(&plan).contains(&"UserService.lookup"));
    assert!(titles(&plan).contains(&"MailNotifier.send"));
    assert_eq!(
        wired(&plan, "Notifier.send", "MailNotifier.send"),
        Some(WireKind::Ambiguous)
    );
}

#[test]
fn callbacks_of_parameters_are_unknown_targets() {
    let dir = fixture();
    let project = Project::read(dir.path());
    let plan = neighbourhood(&project, id(&project, "join"), &Opened::default());
    let join = &plan.nodes[plan.focus.unwrap()];
    assert!(join.unknown_target);
    assert!(titles(&plan).contains(&"mediasoup"));
    assert_eq!(
        join.detail,
        "(id: string, done: () => void) → Promise<void>"
    );
}

#[test]
fn a_module_plan_frames_its_functions_with_calls_in_and_out_at_the_edges() {
    let dir = fixture();
    let project = Project::read(dir.path());
    let bugs = module(&project, "server/src/org/demo/bugs/", &Opened::default());
    let column = |plan: &Plan, title: &str| {
        plan.nodes
            .iter()
            .find(|n| n.title == title)
            .map(|n| n.column)
            .unwrap_or_else(|| panic!("no {title} in {:?}", titles(plan)))
    };
    // Its own, framed by type, called ones right of their callers.
    assert!(column(&bugs, "BugService.save") > column(&bugs, "BugService.create"));
    let frames: Vec<&str> = bugs.frames.iter().map(|f| f.title.as_str()).collect();
    assert!(frames.contains(&"class BugService") && frames.contains(&"class MailNotifier"));
    // What it calls elsewhere, on the right.
    assert!(column(&bugs, "UserService.find") > column(&bugs, "BugService.save"));
    assert!(column(&bugs, "org.slf4j") > column(&bugs, "BugService.create"));
    // And a file's, with its callers from elsewhere on the left.
    let users = module(
        &project,
        "server/src/org/demo/users/UserService.java",
        &Opened::default(),
    );
    assert_eq!(column(&users, "BugService.create"), -1);
}

#[test]
fn a_side_past_eight_folds_into_more() {
    let dir = tempfile::tempdir().unwrap();
    let mut source = String::from("fn hub() {}\n");
    for n in 0..11 {
        source.push_str(&format!("fn caller{n}() {{ hub(); }}\n"));
    }
    std::fs::write(dir.path().join("lib.rs"), source).unwrap();
    let project = Project::read(dir.path());
    let plan = neighbourhood(&project, id(&project, "hub"), &Opened::default());
    let callers = plan.nodes.iter().filter(|n| n.column == -1).count();
    assert_eq!(callers, MAX_SIDE, "seven callers and a +N more");
    assert!(titles(&plan).contains(&"+4 more"));
    let mut opened = Opened::default();
    opened.more.insert(format!(
        "{}@-1",
        function_key(&project, id(&project, "hub"))
    ));
    let all = neighbourhood(&project, id(&project, "hub"), &opened);
    assert_eq!(all.nodes.iter().filter(|n| n.column == -1).count(), 11);
}
