//! Updates: the version running, checking for a newer one, and whether
//! Jig looks by itself.

use gpui_kit::component::Disableable as _;
use gpui_kit::*;

use super::SettingsWindow;
use super::ui::{self, Row, Section};
use jig_settings as settings;
use jig_updates::{self as update, RestartToUpdate, State};

impl SettingsWindow {
    pub(super) fn updates_sections(&self, cx: &Context<Self>) -> Vec<Section> {
        let state = update::state(cx);
        let (description, warning) = match &state {
            State::Idle => (None, false),
            State::Checking => (Some("Checking…".to_string()), false),
            State::UpToDate => (Some("Up to date.".into()), false),
            State::Downloading(version) => (Some(format!("Downloading Jig {version}…")), false),
            State::Ready { version, .. } => (
                Some(format!(
                    "Jig {version} is downloaded. Restart Jig to update."
                )),
                false,
            ),
            State::Manual { version, reason } => {
                (Some(format!("Jig {version} is available. {reason}")), false)
            }
            State::Failed(error) => (Some(error.clone()), true),
        };
        vec![
            Section::new()
                .row(
                    Row::new("version", format!("Jig {}", update::current_version()))
                        .description_opt(description)
                        .warning(warning)
                        .keywords(["update", "version", "upgrade", "release", "about"])
                        .control(move |_, cx| update_button(&update::state(cx))),
                )
                .row(
                    Row::new("automatic-updates", "Check for updates automatically")
                        .description(
                            "Once a day, downloading any it finds. Nothing is installed until \
                             you restart Jig.",
                        )
                        .keywords(["update", "automatic", "download"])
                        .control(ui::switch(
                            "automatic-updates",
                            |cx| settings::get(cx).updates.automatic,
                            |on, cx| settings::update(cx, |s| s.updates.automatic = on),
                        )),
                ),
        ]
    }
}

/// Check Now, or what to do with what the check found.
fn update_button(state: &State) -> AnyElement {
    match state {
        State::Ready { .. } => ui::button("restart-to-update", "Restart to Update")
            .on_click(|_, _, cx| {
                super::send_to_workspace(Box::new(RestartToUpdate), cx);
            })
            .into_any_element(),
        State::Manual { .. } => ui::button("download-update", "Download")
            .on_click(|_, _, cx| update::open_releases_page(cx))
            .into_any_element(),
        State::Checking | State::Downloading(_) => ui::button("check-for-updates", "Check Now")
            .disabled(true)
            .into_any_element(),
        _ => ui::button("check-for-updates", "Check Now")
            .on_click(|_, _, cx| update::check(true, cx))
            .into_any_element(),
    }
}
