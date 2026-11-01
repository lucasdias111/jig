use super::*;

fn sample() -> Domains {
    toml::from_str(
        r#"
[[domain]]
name = "Bugs"
purpose = "Report bugs."
offers = ["Bug form"]
why = "They handle bugs."
members = ["server/src/org/demo/bugs/", "server/src/org/demo/users/UserService.java#UserService"]

[[domain]]
name = "Rooms"
purpose = "Join rooms."
members = ["client/"]

[[flow]]
from = "Rooms"
to = "Bugs"
label = "reports problems to"

[shared]
members = ["client/src/net/", "server/src/org/demo/bugs/Notifier.java"]
"#,
    )
    .unwrap()
}

#[test]
fn the_closest_member_is_a_files_home() {
    let domains = sample();
    assert_eq!(
        domains.home_of("server/src/org/demo/bugs/BugService.java"),
        Home::Domain(0)
    );
    // A file over its folder, a deeper folder over one further out.
    assert_eq!(
        domains.home_of("server/src/org/demo/bugs/Notifier.java"),
        Home::Shared
    );
    assert_eq!(domains.home_of("client/src/net/socket.ts"), Home::Shared);
    assert_eq!(domains.home_of("client/src/rooms.ts"), Home::Domain(1));
    // A type of its own, apart from its file.
    assert_eq!(
        domains.home_of("server/src/org/demo/users/UserService.java#UserService"),
        Home::Domain(0)
    );
    assert_eq!(
        domains.home_of("server/src/org/demo/users/UserService.java"),
        Home::Unsorted
    );
    // A type nothing names follows its file.
    assert_eq!(domains.home_of("client/src/rooms.ts#Room"), Home::Domain(1));
}

#[test]
fn written_as_it_was_read() {
    let domains = sample();
    let text = domains.to_toml();
    assert!(text.contains("members = [\n  \"server/src/org/demo/bugs/\",\n"));
    assert!(text.contains("offers = [\"Bug form\"]"));
    let again: Domains = toml::from_str(&text).unwrap();
    assert_eq!(again, domains);
}

#[test]
fn edits_keep_names_and_flows_together() {
    let mut domains = sample();
    domains.rename(0, "Bug Reports");
    assert_eq!(domains.flows[0].to, "Bug Reports");
    // A name another domain has is refused.
    domains.rename(0, "Rooms");
    assert_eq!(domains.domains[0].name, "Bug Reports");

    domains.move_member("client/src/rooms.ts", Home::Domain(0));
    assert_eq!(domains.home_of("client/src/rooms.ts"), Home::Domain(0));
    domains.move_member("client/src/rooms.ts", Home::Domain(1));
    // Back where its folder puts it, and not named twice.
    assert!(
        !domains.domains[1]
            .members
            .contains(&"client/src/rooms.ts".into())
    );
    assert_eq!(domains.home_of("client/src/rooms.ts"), Home::Domain(1));

    domains.merge(1, 0);
    assert_eq!(domains.domains.len(), 1);
    assert_eq!(domains.home_of("client/src/rooms.ts"), Home::Domain(0));
    // The flow between them is now within one domain: gone.
    assert!(domains.flows.is_empty());
}

#[test]
fn markdown_lists_names_purposes_offers_and_flows() {
    let text = sample().markdown("demo");
    assert!(
        text.starts_with("# demo: domains\n\n## Bugs\n\nReport bugs.\n\nOffers:\n- Bug form\n")
    );
    assert!(text.contains("- Rooms reports problems to Bugs\n"));
}
