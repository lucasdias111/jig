use super::*;

fn saved() -> Domains {
    toml::from_str(
        r#"
[[domain]]
name = "Bookings"
purpose = "Book places."
members = ["src/bookings/"]

[[domain]]
name = "Events"
purpose = "List events."
members = ["src/events/"]

[[flow]]
from = "Events"
to = "Bookings"
label = "supplies events to"

[shared]
members = ["src/util.ts"]
"#,
    )
    .unwrap()
}

#[test]
fn the_toml_is_read_from_its_fenced_block() {
    let reply = "Here you go:\n\n```toml\n[[domain]]\nname = \"A\"\n```\nDone.";
    assert_eq!(fenced_toml(reply), "[[domain]]\nname = \"A\"");
    assert_eq!(fenced_toml("[[domain]]"), "[[domain]]");
}

#[test]
fn index_proposes_what_the_agent_answered() {
    let reply = format!("```toml\n{}\n```", saved().to_toml());
    let proposal = Action::Index.proposal(&reply, None).unwrap();
    assert_eq!(proposal, saved());
    assert!(Action::Index.proposal("No idea.", None).is_err());
}

#[test]
fn update_places_only_unsorted_files_and_never_moves_the_rest() {
    let reply = r#"```toml
[[place]]
file = "src/gifts/Gift.ts"
domain = "Gift Cards"

[[place]]
file = "src/gifts/log.ts"
domain = "Shared"

[[place]]
file = "src/events/Event.ts"
domain = "Bookings"

[[new_domain]]
name = "Gift Cards"
purpose = "Give a gift."
offers = ["Gift cards"]
why = "Gifts."

[[new_domain]]
name = "Nothing Placed"

[[new_flow]]
from = "Gift Cards"
to = "Bookings"
label = "pays for"
```"#;
    let unsorted = vec!["src/gifts/Gift.ts".to_string(), "src/gifts/log.ts".into()];
    let proposal = Action::Update(unsorted)
        .proposal(reply, Some(&saved()))
        .unwrap();
    let names: Vec<&str> = proposal.domains.iter().map(|d| d.name.as_str()).collect();
    assert_eq!(names, ["Bookings", "Events", "Gift Cards"]);
    assert_eq!(proposal.home_of("src/gifts/Gift.ts"), Home::Domain(2));
    assert_eq!(proposal.home_of("src/gifts/log.ts"), Home::Shared);
    // Confirmed files stay where they were.
    assert_eq!(proposal.home_of("src/events/Event.ts"), Home::Domain(1));
    assert_eq!(proposal.domains[..2], saved().domains[..]);
    assert_eq!(proposal.flows.len(), 2);
}

#[test]
fn a_split_replaces_the_domain_where_it_was() {
    let reply = r#"```toml
[[domain]]
name = "Reservations"
members = ["src/bookings/reserve/"]

[[domain]]
name = "Refunds"
members = ["src/bookings/refund/"]

[[flow]]
from = "Events"
to = "Reservations"
label = "supplies events to"

[[flow]]
from = "Reservations"
to = "Refunds"
label = "cancels into"
```"#;
    let proposal = Action::Split("Bookings".into())
        .proposal(reply, Some(&saved()))
        .unwrap();
    let names: Vec<&str> = proposal.domains.iter().map(|d| d.name.as_str()).collect();
    assert_eq!(names, ["Reservations", "Refunds", "Events"]);
    assert_eq!(proposal.flows.len(), 2);
    assert!(proposal.flows.iter().all(|f| f.to != "Bookings"));
}

#[test]
fn requests_are_known_by_what_they_do() {
    let request = |kind: Asking, detail: &str| PermissionRequest {
        id: "1".into(),
        title: "Bash".into(),
        detail: detail.into(),
        kind,
        always: None,
        task: None,
    };
    let reads = |request| only_reads(&request, Path::new("/p"));
    assert!(reads(request(Asking::Command, "ls 2>/dev/null")));
    assert!(!reads(request(Asking::Command, "rm -rf src")));
    assert!(reads(request(Asking::Read, "src/main.rs")));
    assert!(reads(request(
        Asking::Subagent { explores: true },
        "Find the routes"
    )));
    assert!(!reads(request(
        Asking::Subagent { explores: false },
        "Do it"
    )));
    assert!(!reads(request(Asking::Web, "https://example.com")));
    assert!(!reads(request(Asking::Other, "ls")));
}
