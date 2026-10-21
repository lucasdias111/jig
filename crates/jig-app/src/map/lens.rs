//! What the agent draws on the Map: the Lens. Each call of one of the
//! Map's tools (`jig_ai::map`) is one drawing: a highlight, a note, a
//! trace or tour, a group, ghosts it proposes, or purposes and labels it
//! describes. The agent names places as `path#Type.method`,
//! `path#function`, `path`, `folder/` or `domain:Name`; each is resolved
//! against the project's index, accepting any real file, and a name that
//! isn't found comes back with names like it. `mcp` serves the tools; the
//! app reads each call back from the agent's own report of it, so a
//! drawing belongs to its conversation and comes back with it.

pub mod mcp;
pub mod pins;
pub mod purposes;

use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::domains::Domains;
use super::index::{FnId, Project};

/// What a drawing is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    /// Places lit up.
    Highlight,
    /// A remark on one place.
    Note,
    /// A path through places, in order, each step labelled.
    Trace,
    /// A trace the camera flies through.
    Tour,
    /// Places circled together, with a label.
    Group,
    /// Ghost boxes and arrows: a change the agent suggests.
    Propose,
    /// Purposes for places and labels for arrows, to keep or not.
    Describe,
}

impl Kind {
    /// Its steps are in order, to step through.
    pub fn stepped(self) -> bool {
        matches!(self, Kind::Trace | Kind::Tour)
    }

    pub fn name(self) -> &'static str {
        match self {
            Kind::Highlight => "Highlight",
            Kind::Note => "Note",
            Kind::Trace => "Trace",
            Kind::Tour => "Tour",
            Kind::Group => "Group",
            Kind::Propose => "Proposal",
            Kind::Describe => "Descriptions",
        }
    }
}

/// A place the agent named, with what it says there.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Step {
    /// As the agent named it.
    pub place: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub label: String,
}

/// An arrow between two places, or a place and a ghost.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Link {
    pub from: String,
    pub to: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub label: String,
}

/// A box the agent proposes, drawn as a ghost near a real place.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ghost {
    pub name: String,
    /// The real place it goes by.
    pub near: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub purpose: String,
}

/// One call of the Map's tools.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Drawing {
    pub kind: Kind,
    #[serde(default)]
    pub title: String,
    /// The places, in order: lit, noted, traced, grouped or described.
    #[serde(default, rename = "step", skip_serializing_if = "Vec::is_empty")]
    pub steps: Vec<Step>,
    /// Arrows lit, proposed or labelled.
    #[serde(default, rename = "link", skip_serializing_if = "Vec::is_empty")]
    pub links: Vec<Link>,
    #[serde(default, rename = "ghost", skip_serializing_if = "Vec::is_empty")]
    pub ghosts: Vec<Ghost>,
}

impl Drawing {
    /// A line for the transcript: "Trace: Booking a class · 10 steps".
    pub fn summary(&self) -> String {
        let count = |n: usize, what: &str| match (n, what) {
            (1, _) => format!("1 {what}"),
            (n, "box") => format!("{n} boxes"),
            (n, _) => format!("{n} {what}s"),
        };
        let what = match self.kind {
            Kind::Trace | Kind::Tour => count(self.steps.len(), "step"),
            Kind::Propose => count(self.ghosts.len(), "box"),
            Kind::Describe => count(self.steps.len() + self.links.len(), "description"),
            Kind::Note => self
                .steps
                .first()
                .map_or(String::new(), |s| s.place.clone()),
            _ => count(self.steps.len(), "place"),
        };
        match (self.title.trim(), what.is_empty()) {
            ("", _) => format!("{}: {what}", self.kind.name()),
            (title, true) => format!("{}: {title}", self.kind.name()),
            (title, false) => format!("{}: {title} · {what}", self.kind.name()),
        }
    }

    /// Every place it names, ghosts' names left out.
    pub fn places(&self) -> Vec<&str> {
        let ghost = |name: &str| self.ghosts.iter().any(|g| g.name == name);
        let mut places: Vec<&str> = self.steps.iter().map(|s| s.place.as_str()).collect();
        for link in &self.links {
            for end in [&link.from, &link.to] {
                if !ghost(end) {
                    places.push(end);
                }
            }
        }
        places.extend(self.ghosts.iter().map(|g| g.near.as_str()));
        places
    }
}

/// What a call of the Map's tools does.
#[derive(Clone, Debug, PartialEq)]
pub enum Call {
    Draw(Drawing),
    /// Everything drawn so far in the conversation goes.
    Clear,
}

/// A tool call as a drawing: `tool` is the Map's name for it
/// (`map_trace`), `input` what the agent gave it.
pub fn call(tool: &str, input: &Value) -> Result<Call, String> {
    let text = |value: &Value| value.as_str().unwrap_or_default().trim().to_string();
    // The agent may name places with either word.
    let place = |value: &Value| {
        if value.is_string() {
            text(value)
        } else {
            text(
                [&value["place"], &value["node"], &value["name"]]
                    .into_iter()
                    .find(|v| v.is_string())
                    .unwrap_or(&Value::Null),
            )
        }
    };
    let list = |name: &str| -> Vec<&Value> {
        input[name]
            .as_array()
            .map(|items| items.iter().collect())
            .unwrap_or_default()
    };
    let places = |names: &[&str]| -> Vec<Step> {
        names
            .iter()
            .flat_map(|name| list(name))
            .map(|value| Step {
                place: place(value),
                label: text(&value["label"]),
            })
            .filter(|step| !step.place.is_empty())
            .collect()
    };
    let links = |name: &str| -> Vec<Link> {
        list(name)
            .into_iter()
            .map(|link| Link {
                from: text(&link["from"]),
                to: text(&link["to"]),
                label: text(&link["label"]),
            })
            .filter(|link| !link.from.is_empty() && !link.to.is_empty())
            .collect()
    };
    let title = text(&input["title"]);
    let drawing = |kind: Kind, steps: Vec<Step>| Drawing {
        kind,
        title: title.clone(),
        steps,
        links: Vec::new(),
        ghosts: Vec::new(),
    };
    let drawing = match tool {
        "map_clear" => return Ok(Call::Clear),
        "map_highlight" => Drawing {
            links: links("arrows"),
            ..drawing(Kind::Highlight, places(&["places", "nodes"]))
        },
        "map_note" => {
            let note = text(&input["text"]);
            Drawing {
                title: note.clone(),
                ..drawing(
                    Kind::Note,
                    vec![Step {
                        place: place(input),
                        label: note,
                    }],
                )
            }
        }
        "map_trace" => drawing(Kind::Trace, places(&["steps"])),
        "map_tour" => drawing(Kind::Tour, places(&["steps"])),
        "map_group" => drawing(Kind::Group, places(&["places", "nodes"])),
        "map_propose" => Drawing {
            links: links("arrows"),
            ghosts: list("boxes")
                .into_iter()
                .map(|ghost| Ghost {
                    name: text(&ghost["name"]),
                    near: text(&ghost["near"]),
                    purpose: text(&ghost["purpose"]),
                })
                .filter(|ghost| !ghost.name.is_empty() && !ghost.near.is_empty())
                .collect(),
            ..drawing(Kind::Propose, Vec::new())
        },
        "map_describe" => Drawing {
            steps: list("purposes")
                .into_iter()
                .map(|purpose| Step {
                    place: place(purpose),
                    label: text(&purpose["purpose"]),
                })
                .filter(|step| !step.place.is_empty() && !step.label.is_empty())
                .collect(),
            links: links("labels"),
            ..drawing(Kind::Describe, Vec::new())
        },
        other => return Err(format!("There's no tool {other}.")),
    };
    let empty = drawing.steps.is_empty() && drawing.ghosts.is_empty() && drawing.links.is_empty();
    if empty {
        return Err("Nothing to draw: name at least one place.".into());
    }
    Ok(Call::Draw(drawing))
}

/// Where a named place is in the project.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Place {
    /// A domain, by its name.
    Domain(String),
    /// A folder, `dir/`.
    Folder(String),
    /// A file, read by the index or not, maybe a type in it, maybe a
    /// function (`Owner.name` or `name`).
    File {
        path: String,
        owner: Option<String>,
        function: Option<String>,
    },
    /// A box of the user's sketch (`sketch:<name>/<box>`), by their ids.
    Sketch { sketch: String, shape: String },
}

impl Place {
    /// As Site names its boxes: `dir/`, `path` or `path#Type`.
    pub fn site_place(&self) -> Option<String> {
        match self {
            Place::Domain(_) | Place::Sketch { .. } => None,
            Place::Folder(dir) => Some(dir.clone()),
            Place::File {
                path,
                owner: Some(owner),
                ..
            } => Some(format!("{path}#{owner}")),
            Place::File { path, .. } => Some(path.clone()),
        }
    }

    /// The file it's in, if it's in one.
    pub fn file(&self) -> Option<&str> {
        match self {
            Place::File { path, .. } => Some(path),
            _ => None,
        }
    }

    /// The function it is, in `project`.
    pub fn function_in(&self, project: &Project) -> Option<FnId> {
        let Place::File {
            path,
            function: Some(title),
            ..
        } = self
        else {
            return None;
        };
        let file = project
            .files
            .iter()
            .position(|file| slashed(&file.path) == *path)?;
        project
            .functions
            .iter()
            .position(|f| f.file == file && f.title() == *title)
    }
}

/// `place` in `project`, or names like it to suggest.
pub fn resolve(
    project: &Project,
    domains: Option<&Domains>,
    place: &str,
) -> Result<Place, Vec<String>> {
    let named = normal(project, place);
    if named.starts_with(super::sketch::PREFIX) {
        return resolve_sketch(&project.root, &named);
    }
    if let Some(name) = named.strip_prefix("domain:") {
        let name = name.trim();
        return match domains
            .and_then(|d| d.domains.iter().find(|d| d.name.eq_ignore_ascii_case(name)))
        {
            Some(domain) => Ok(Place::Domain(domain.name.clone())),
            None => Err(domains
                .map(|d| {
                    d.domains
                        .iter()
                        .map(|d| format!("domain:{}", d.name))
                        .collect()
                })
                .unwrap_or_default()),
        };
    }
    let (path, inner) = match named.split_once('#') {
        Some((path, inner)) => (path.to_string(), Some(inner.trim_end_matches("()"))),
        None => (named.clone(), None),
    };
    let read = project.files.iter().position(|f| slashed(&f.path) == path);
    let real = read.is_some()
        || project.others.iter().any(|f| slashed(f) == path)
        || (!path.ends_with('/') && project.root.join(&path).is_file());
    if !real {
        let dir = format!("{}/", path.trim_end_matches('/'));
        let folder = !path.is_empty()
            && (project
                .files
                .iter()
                .any(|f| slashed(&f.path).starts_with(&dir))
                || project.others.iter().any(|f| slashed(f).starts_with(&dir))
                || project.root.join(&dir).is_dir());
        if folder && inner.is_none() {
            return Ok(Place::Folder(dir));
        }
        return Err(suggest(project, &path, inner));
    }
    let file = |owner: Option<String>, function: Option<String>| Place::File {
        path: path.clone(),
        owner,
        function,
    };
    let (Some(inner), Some(ix)) = (inner.filter(|i| !i.is_empty()), read) else {
        // A file as a whole, or one the index can't read, by any name in
        // it: its box is the file's.
        return Ok(file(None, None));
    };
    let functions: Vec<&super::index::Func> =
        project.functions.iter().filter(|f| f.file == ix).collect();
    // `#L12`: the function on that line.
    if let Some(line) = inner
        .strip_prefix('L')
        .and_then(|line| line.parse::<u32>().ok())
        && let Some(id) = project.function_on_line(&project.files[ix].path, line.saturating_sub(1))
    {
        let function = &project.functions[id];
        return Ok(file(function.outline.owner.clone(), Some(function.title())));
    }
    let found = functions
        .iter()
        .find(|f| f.title() == inner)
        .or_else(|| functions.iter().find(|f| f.short() == inner))
        .or_else(|| {
            // `Type.method` where the index has it under another owner.
            let short = inner.rsplit(['.', ':']).next()?;
            functions.iter().find(|f| f.short() == short)
        });
    if let Some(function) = found {
        return Ok(file(function.outline.owner.clone(), Some(function.title())));
    }
    let types = &project.files[ix].types;
    if functions
        .iter()
        .any(|f| f.outline.owner.as_deref() == Some(inner))
        || types.iter().any(|ty| ty.name == inner)
    {
        return Ok(file(Some(inner.to_string()), None));
    }
    // The file's functions, those most like the name first.
    let wanted = inner
        .rsplit(['.', ':'])
        .next()
        .unwrap_or(inner)
        .to_lowercase();
    let mut like: Vec<&&super::index::Func> = functions.iter().collect();
    like.sort_by_key(|f| std::cmp::Reverse(likeness(&f.short().to_lowercase(), &wanted)));
    Err(like
        .into_iter()
        .take(8)
        .map(|f| format!("{path}#{}", f.title()))
        .collect())
}

/// A box of a sketch saved in the project at `root`, by its id or label;
/// else the sketches' boxes, to suggest.
fn resolve_sketch(root: &Path, name: &str) -> Result<Place, Vec<String>> {
    let sketches = super::sketch::read_all(root);
    let all = || {
        sketches
            .iter()
            .flat_map(|s| {
                s.boxes
                    .iter()
                    .map(|b| super::sketch::place_name(&s.name, &b.id))
            })
            .take(12)
            .collect()
    };
    let Some((wanted, shape)) = super::sketch::parse_place(name) else {
        return Err(all());
    };
    let same = |a: &str, b: &str| super::sketch::slug(a, "") == super::sketch::slug(b, "");
    let found = sketches
        .iter()
        .find(|s| s.name == wanted || same(&s.title, &wanted))
        .and_then(|s| {
            let found = s
                .boxes
                .iter()
                .find(|b| b.id == shape || same(&b.label, &shape))?;
            Some(Place::Sketch {
                sketch: s.name.clone(),
                shape: found.id.clone(),
            })
        });
    found.ok_or_else(all)
}

/// How alike two names are: the start they share, more when one holds the
/// other.
fn likeness(a: &str, b: &str) -> usize {
    let shared = a.chars().zip(b.chars()).take_while(|(x, y)| x == y).count();
    let held = if !b.is_empty() && (a.contains(b) || b.contains(a)) {
        b.len().min(a.len())
    } else {
        0
    };
    shared + held
}

/// A name as the index keeps paths: relative to the project, with `/`,
/// without `./` or a line.
fn normal(project: &Project, place: &str) -> String {
    let mut place = place.trim().trim_matches('`').replace('\\', "/");
    let root = slashed(&project.root);
    if let Some(rest) = place.strip_prefix(&format!("{}/", root.trim_end_matches('/'))) {
        place = rest.to_string();
    }
    while let Some(rest) = place.strip_prefix("./") {
        place = rest.to_string();
    }
    // `path:12` is the function on that line.
    if !place.contains('#')
        && let Some((path, line)) = place.rsplit_once(':')
        && line.chars().all(|c| c.is_ascii_digit())
        && !line.is_empty()
    {
        place = format!("{path}#L{line}");
    }
    place
}

/// Up to six places named like what was asked for.
fn suggest(project: &Project, path: &str, inner: Option<&str>) -> Vec<String> {
    let last = |text: &str| {
        text.rsplit(['.', '/', '#'])
            .find(|part| !part.is_empty())
            .unwrap_or_default()
            .to_lowercase()
    };
    let mut like: Vec<String> = Vec::new();
    // Files named the same, wherever they are.
    let file_name = path.rsplit('/').next().unwrap_or(path).to_lowercase();
    if !file_name.is_empty() {
        for file in project
            .files
            .iter()
            .map(|f| slashed(&f.path))
            .chain(project.others.iter().map(|f| slashed(f)))
        {
            if file.to_lowercase().ends_with(&format!("/{file_name}")) || file == file_name {
                like.push(match inner {
                    Some(inner) => format!("{file}#{inner}"),
                    None => file,
                });
            }
        }
    }
    let wanted = last(inner.unwrap_or(path));
    if !wanted.is_empty() {
        for function in &project.functions {
            if like.len() >= 6 {
                break;
            }
            if function.short().to_lowercase() == wanted {
                like.push(format!(
                    "{}#{}",
                    slashed(&project.files[function.file].path),
                    function.title()
                ));
            }
        }
    }
    like.truncate(6);
    like
}

/// A path with `/`, as places are named.
pub fn slashed(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

/// Each place `drawing` names resolved, or why not: every name that isn't
/// found, with names like it.
pub fn check(
    project: &Project,
    domains: Option<&Domains>,
    drawing: &Drawing,
) -> Result<(), String> {
    let mut missing = Vec::new();
    for place in drawing.places() {
        if let Err(like) = resolve(project, domains, place) {
            missing.push(if like.is_empty() {
                format!("{place} (nothing like it)")
            } else {
                format!("{place} (did you mean {})", like.join(", "))
            });
        }
    }
    if missing.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "Nothing was drawn: these don't name a place in the project: {}. Name each as path#Type.method, path#function, path, folder/, domain:Name or sketch:<sketch>/<box>, paths relative to the project, and draw again.",
            missing.join("; ")
        ))
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use serde_json::json;

    use super::*;
    use crate::map::index::tests::fixture;

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
            resolve(&project, None, &format!("{users}#UserService.find")),
            Ok(Place::File {
                path: users.into(),
                owner: Some("UserService".into()),
                function: Some("UserService.find".into())
            })
        );
        assert_eq!(
            resolve(&project, None, &format!("./{users}#find")),
            resolve(&project, None, &format!("{users}#UserService.find")),
            "by its short name too"
        );
        assert!(matches!(
            resolve(&project, None, &format!("{users}#UserService")),
            Ok(Place::File {
                owner: Some(_),
                function: None,
                ..
            })
        ));
        assert_eq!(
            resolve(&project, None, "server/src/org/demo/"),
            Ok(Place::Folder("server/src/org/demo/".into()))
        );
        assert_eq!(
            resolve(&project, None, "server/src/org/demo"),
            Ok(Place::Folder("server/src/org/demo/".into())),
            "without its slash"
        );
        let wrong = resolve(&project, None, &format!("{users}#UserService.remove")).unwrap_err();
        assert!(
            wrong.contains(&format!("{users}#UserService.find")),
            "{wrong:?}"
        );
        let moved = resolve(&project, None, "src/UserService.java#find").unwrap_err();
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
        let error = check(&project, None, &drawing).unwrap_err();
        assert!(error.contains("nowhere.rs"), "{error}");
        assert!(!error.contains("demo/ ("), "{error}");
    }
}
