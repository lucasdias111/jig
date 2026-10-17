//! Providers: where the agent's models come from, connected with a key or,
//! for a server on this computer, a switch; and the agent's model from
//! them.

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::{Sizable as _, h_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use jig_ai::{Config, ProviderConfig, ProviderTemplate, TEMPLATES, template_named};

use super::ui::{self, Row, Section};
use super::{SettingsWindow, send_to_workspace};
use crate::providers::{self, ModelList};
use crate::workspace::EditModelConfig;

/// Marks an agent model OpenCode knows by itself, from an older file's
/// `agent_model`.
const OPENCODE_OWN: &str = "opencode:";

impl SettingsWindow {
    /// Providers: the accounts and local servers the models come from.
    /// Picking the agent's model is the Agent page's.
    pub(super) fn providers_sections(&self, cx: &Context<Self>) -> Vec<Section> {
        let config = match providers::config(cx) {
            Ok(config) => config,
            Err(error) => {
                return vec![
                    Section::new()
                        .row(
                            Row::note(
                                "providers-error",
                                format!("Couldn't read your providers. {error}"),
                            )
                            .warning(true),
                        )
                        .row(providers_file_row()),
                ];
            }
        };
        let mut sections = Vec::new();
        if let Some(error) = self.providers_error.clone() {
            sections.push(Section::new().row(Row::note("providers-error", error).warning(true)));
        }
        sections.push(self.providers_section(&config, cx));
        let hand_added: Vec<Row> = config
            .providers
            .iter()
            .filter(|provider| template_named(&provider.name).is_none())
            .map(|provider| other_provider_row(provider, cx))
            .collect();
        sections.push(
            Section::titled("Other Providers")
                .rows(hand_added)
                .row(providers_file_row()),
        );
        sections
    }

    /// The model OpenCode runs on, from what the connected providers offer.
    pub(super) fn opencode_model_row(&self, config: &Config, cx: &Context<Self>) -> Row {
        let this = cx.entity().downgrade();
        let offered = offered_models(config, cx);
        if offered.is_empty() && config.agent.as_ref().is_none_or(|id| id.is_empty()) {
            return Row::note(
                "no-models",
                "Connect a provider under Providers. The agent starts on one of its fast models, which you can change here.",
            );
        }
        let mut agent_options = Vec::new();
        let agent = match (
            config.agent.as_ref().filter(|id| !id.is_empty()),
            &config.agent_model,
        ) {
            (Some(id), _) => id.clone(),
            (None, Some(native)) => {
                let id = format!("{OPENCODE_OWN}{native}");
                agent_options.push((
                    id.clone().into(),
                    format!("{native} (from OpenCode)").into(),
                ));
                id
            }
            (None, None) => {
                agent_options.push(("".into(), "Pick a model".into()));
                String::new()
            }
        };
        agent_options.extend(offered);
        keep_current(&mut agent_options, &agent, config);

        Row::new("agent-model", "Model")
            .description("What OpenCode runs on, from your providers.")
            .keywords(["model", "agent", "opencode"])
            .control(ui::dropdown(
                "agent-model",
                agent_options,
                move |_| agent.clone().into(),
                move |id, cx| {
                    if id.is_empty() || id.starts_with(OPENCODE_OWN) {
                        return;
                    }
                    let result = providers::set_agent(&id, cx);
                    this.update(cx, |this, cx| this.report(result, cx)).ok();
                },
            ))
    }

    /// Every major provider, with a key field or a switch to connect it.
    fn providers_section(&self, config: &Config, cx: &Context<Self>) -> Section {
        let rows = TEMPLATES.iter().map(|template| {
            let provider = config.provider(template.name).cloned();
            match self
                .key_inputs
                .iter()
                .find(|(keyed, _)| keyed.name == template.name)
            {
                Some((_, input)) => keyed_row(template, provider, input.clone(), cx),
                None => local_row(template, provider, cx),
            }
        });
        Section::new()
            .footer("Keys are kept in the system keychain, never in config.toml.")
            .rows(rows)
            .row(refresh_row())
    }
}

/// How the model lists name a provider.
fn provider_label(name: &str) -> &str {
    template_named(name).map_or(name, |template| template.label)
}

/// Every model the connected providers offer, as (`provider/model`, label).
fn offered_models(config: &Config, cx: &App) -> Vec<(SharedString, SharedString)> {
    let mut offered = Vec::new();
    for provider in &config.providers {
        if let Some(ModelList::Loaded(models)) = providers::models(&provider.name, cx) {
            for model in models {
                offered.push((
                    format!("{}/{model}", provider.name).into(),
                    format!("{} · {model}", provider_label(&provider.name)).into(),
                ));
            }
        }
    }
    offered
}

/// Keep the model in use in its list while models load, or if the
/// provider stopped offering it.
fn keep_current(options: &mut Vec<(SharedString, SharedString)>, id: &str, config: &Config) {
    if id.is_empty() || options.iter().any(|(value, _)| value == id) {
        return;
    }
    let label = match jig_ai::split_model(id) {
        Some((name, model)) if config.provider(name).is_some() => {
            format!("{} · {model}", provider_label(name))
        }
        _ => format!("{id} (not connected)"),
    };
    options.insert(0, (id.to_string().into(), label.into()));
}

/// What a provider row says, and whether it's fine.
fn connection_status(provider: &ProviderConfig, cx: &App) -> (String, bool) {
    if !providers::is_ready(provider, cx) {
        return ("Paste a key to connect.".into(), false);
    }
    let source = providers::api_key(provider, cx)
        .map(|(_, source)| format!(" · {}", source.label()))
        .unwrap_or_default();
    match providers::models(&provider.name, cx) {
        Some(ModelList::Loaded(models)) => {
            (format!("Connected · {} models{source}", models.len()), true)
        }
        Some(ModelList::Failed(error)) => (error, false),
        Some(ModelList::Loading) | None => (format!("Loading models…{source}"), true),
    }
}

/// A provider that takes a key: the key field until it works, then
/// Disconnect.
fn keyed_row(
    template: &'static ProviderTemplate,
    provider: Option<ProviderConfig>,
    input: Entity<InputState>,
    cx: &Context<SettingsWindow>,
) -> Row {
    let this = cx.entity().downgrade();
    let env = template
        .api_key_env
        .filter(|var| std::env::var(var).is_ok_and(|key| !key.trim().is_empty()));
    let (description, ok) = match &provider {
        Some(provider) => {
            let (status, ok) = connection_status(provider, cx);
            (Some(status), ok)
        }
        None => (None, true),
    };
    let ready = provider
        .as_ref()
        .is_some_and(|provider| providers::is_ready(provider, cx));
    let connected = provider.is_some();
    Row::new(template.name, template.label)
        .description_opt(description)
        .warning(!ok)
        .keywords([template.name, "key", "api", "token", "connect", "provider"])
        .control(move |_, _| {
            let disconnect = this.clone();
            let use_env = this.clone();
            h_flex()
                .gap_2()
                .items_center()
                .when(!ready, |row| {
                    row.child(Input::new(&input).small().mask_toggle().w(px(190.)))
                })
                .when_some(env.filter(|_| !connected), |row, var| {
                    row.child(
                        Button::new(SharedString::from(format!("env-{}", template.name)))
                            .label(format!("Use ${var}"))
                            .small()
                            .ghost()
                            .on_click(move |_, _, cx| {
                                let result = providers::connect(template, None, cx);
                                use_env.update(cx, |this, cx| this.report(result, cx)).ok();
                            }),
                    )
                })
                .when(connected, |row| {
                    row.child(
                        ui::button(format!("disconnect-{}", template.name), "Disconnect").on_click(
                            move |_, _, cx| {
                                let result = providers::disconnect(template.name, cx);
                                disconnect
                                    .update(cx, |this, cx| this.report(result, cx))
                                    .ok();
                            },
                        ),
                    )
                })
        })
}

/// A server on this computer: a switch, since it needs no key.
fn local_row(
    template: &'static ProviderTemplate,
    provider: Option<ProviderConfig>,
    cx: &Context<SettingsWindow>,
) -> Row {
    let this = cx.entity().downgrade();
    let (description, ok) = match &provider {
        Some(provider) => connection_status(provider, cx),
        None => (format!("On this computer, at {}.", template.base_url), true),
    };
    Row::new(template.name, template.label)
        .description(description)
        .warning(!ok)
        .keywords([template.name, "local", "connect", "provider"])
        .control(ui::switch(
            format!("provider-{}", template.name),
            move |cx| {
                providers::config(cx).is_ok_and(|config| config.provider(template.name).is_some())
            },
            move |on, cx| {
                let result = if on {
                    providers::connect(template, None, cx)
                } else {
                    providers::disconnect(template.name, cx)
                };
                this.update(cx, |this, cx| this.report(result, cx)).ok();
            },
        ))
}

/// A provider added to `config.toml` by hand, which only the file can
/// change.
fn other_provider_row(provider: &ProviderConfig, cx: &Context<SettingsWindow>) -> Row {
    let this = cx.entity().downgrade();
    let name = provider.name.clone();
    let (status, ok) = connection_status(provider, cx);
    Row::new(format!("other-{name}"), provider.name.clone())
        .description(format!("{} · {status}", provider.base_url))
        .warning(!ok)
        .keywords(["provider", "custom"])
        .control(move |_, _| {
            let this = this.clone();
            let name = name.clone();
            ui::button(format!("remove-{name}"), "Remove").on_click(move |_, _, cx| {
                let result = providers::disconnect(&name, cx);
                this.update(cx, |this, cx| this.report(result, cx)).ok();
            })
        })
}

/// Ask every provider for its models again, which also shows whether its
/// key works.
fn refresh_row() -> Row {
    Row::new("refresh-models", "Models")
        .description("Each provider lists its models once its key works.")
        .keywords(["test", "check", "refresh", "models", "connection"])
        .control(|_, _| {
            Button::new("refresh-models")
                .label("Refresh Models")
                .small()
                .ghost()
                .on_click(|_, _, cx| providers::load_all_models(false, cx))
        })
}

fn providers_file_row() -> Row {
    Row::new("providers-file", "Providers file")
        .description("Add any OpenAI- or Anthropic-compatible server in config.toml.")
        .keywords(["config", "provider", "toml", "custom", "file"])
        .control(|_, _| {
            ui::button("edit-config", "Edit…")
                .on_click(|_, _, cx| send_to_workspace(Box::new(EditModelConfig), cx))
        })
}
