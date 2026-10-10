//! Settings > Agent: what the agent may do besides reading the project and
//! proposing edits, each asked about every time, allowed, or turned off.
//! Kept under `[agent]` in `settings.toml` and applied from its next turn.

use gpui_kit::component::setting::{SettingField, SettingGroup, SettingItem, SettingPage};
use gpui_kit::component::{ActiveTheme as _, Icon};
use gpui_kit::*;
use jig_ai::agent::{Access, Permissions};

use super::SettingsWindow;
use crate::settings;

impl SettingsWindow {
    pub(super) fn agent_page(&self) -> SettingPage {
        let access_item = |label: &'static str,
                           description: &'static str,
                           keywords: &'static [&'static str],
                           get: fn(&Permissions) -> Access,
                           set: fn(&mut Permissions, Access)| {
            let options = Access::ALL
                .into_iter()
                .map(|access| (access.key().into(), access_label(access).into()))
                .collect();
            SettingItem::new(
                label,
                SettingField::dropdown(
                    options,
                    move |cx| get(&settings::get(cx).agent).key().into(),
                    move |key: SharedString, cx| {
                        settings::update(cx, |s| set(&mut s.agent, Access::from_key(&key)))
                    },
                )
                .default_value(get(&Permissions::default()).key()),
            )
            .description(description)
            .keywords(keywords.iter().copied().chain(["agent", "permission"]))
        };

        SettingPage::new("Agent")
            .icon(Icon::default().data(jig_commands::surface::AGENT_ICON))
            .group(
                SettingGroup::new()
                    .title("Permissions")
                    .description(
                        "What the agent may do in Agent mode. Its edits always wait for \
                         your review, and it never touches files outside the project. \
                         Changes apply from its next turn.",
                    )
                    .item(access_item(
                        "Shell commands",
                        "Commands it runs in the project, like cargo test.",
                        &["shell", "bash", "command", "terminal"],
                        |p| p.shell,
                        |p, access| p.shell = access,
                    ))
                    .item(access_item(
                        "Web pages",
                        "Pages it fetches to read, like documentation.",
                        &["web", "fetch", "url", "internet"],
                        |p| p.web_fetch,
                        |p, access| p.web_fetch = access,
                    ))
                    .item(access_item(
                        "Web search",
                        "Searches it runs to find something out.",
                        &["web", "search", "internet"],
                        |p| p.web_search,
                        |p, access| p.web_search = access,
                    ))
                    .item(access_item(
                        "Subagents",
                        "Agents it starts for part of the task. Their edits wait for \
                         review like its own.",
                        &["subagent", "task"],
                        |p| p.subagents,
                        |p, access| p.subagents = access,
                    ))
                    .item(
                        SettingItem::new(
                            "Questions with choices",
                            SettingField::switch(
                                |cx| settings::get(cx).agent.questions,
                                |on, cx| settings::update(cx, |s| s.agent.questions = on),
                            )
                            .default_value(Permissions::default().questions),
                        )
                        .description(
                            "It can ask with options to pick from. Off, it asks in its reply.",
                        )
                        .keywords([
                            "question",
                            "ask",
                            "agent",
                            "permission",
                        ]),
                    )
                    .item(SettingItem::render(|_, _, cx| {
                        div()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child(
                                "Allowed, what it ran still shows in the conversation. \
                                 Asked, Enter allows it once and Esc turns it down.",
                            )
                    })),
            )
    }
}

fn access_label(access: Access) -> &'static str {
    match access {
        Access::Ask => "Ask each time",
        Access::Allow => "Allow",
        Access::Deny => "Don't allow",
    }
}
