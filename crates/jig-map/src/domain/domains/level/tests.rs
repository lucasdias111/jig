use super::*;
use crate::domain::index::tests::fixture;
use crate::domain::site::Show;

fn level_of(project: &Project, domains: &Domains) -> DomainLevel {
    let site = Site::of(project, domains, Show::default());
    DomainLevel::of(project, domains, false, &site)
}

fn domains() -> Domains {
    toml::from_str(
        r#"
[[domain]]
name = "Bug Reports"
purpose = "Report what's broken."
members = ["server/src/org/demo/bugs/", "server/src/org/demo/users/UserService.java#UserService"]

[[domain]]
name = "Rooms"
purpose = "Meet in rooms."
members = ["client/src/rooms.ts"]

[[flow]]
from = "Rooms"
to = "Bug Reports"
label = "reports problems to"

[[flow]]
from = "Bug Reports"
to = "Rooms"
label = "closes rooms in"

[shared]
members = ["client/src/net/"]
"#,
    )
    .unwrap()
}

#[test]
fn regions_hold_their_members_and_shared_and_unsorted_sit_apart() {
    let dir = fixture();
    let project = Project::read(dir.path());
    let level = level_of(&project, &domains());
    let names: Vec<&str> = level.showing().map(|(_, r)| r.name.as_str()).collect();
    assert_eq!(names, ["Bug Reports", "Rooms", "Shared", "Unsorted"]);
    let bugs = &level.regions[0];
    // Its files, and a type with a home of its own.
    assert_eq!(bugs.files, 3);
    assert!(
        bugs.members
            .iter()
            .any(|m| m.place == "server/src/org/demo/users/UserService.java#UserService")
    );
    // UserService.java itself is named by nothing.
    assert_eq!(
        level.unsorted(),
        ["server/src/org/demo/users/UserService.java"]
    );
    // Where Site puts them.
    assert!(
        level
            .regions
            .iter()
            .all(|r| r.rect.width > 0. && r.header > 0.)
    );
    // Shared and Unsorted below every domain.
    let lowest = level.regions[..2]
        .iter()
        .map(|r| r.rect.bottom())
        .fold(0., f32::max);
    assert!(level.regions[2].rect.y > lowest);
    assert!(!level.regions[0].rect.overlaps(&level.regions[1].rect));
}

#[test]
fn one_link_per_pair_carries_both_ways() {
    let dir = fixture();
    let project = Project::read(dir.path());
    let level = level_of(&project, &domains());
    assert_eq!(
        level.links,
        [Link {
            a: 0,
            b: 1,
            forward: vec!["closes rooms in".into()],
            backward: vec!["reports problems to".into()],
        }]
    );
}

#[test]
fn more_code_makes_a_bigger_region() {
    let dir = fixture();
    let project = Project::read(dir.path());
    let level = level_of(&project, &domains());
    assert!(level.regions[0].weight > level.regions[1].weight);
    let area = |r: &Region| r.rect.width * r.rect.height;
    assert!(area(&level.regions[0]) >= area(&level.regions[1]));
}
