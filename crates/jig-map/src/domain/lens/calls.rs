//! A call of one of the Map's tools, read as a drawing.

use super::*;

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
