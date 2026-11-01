//! The AI providers (`config.toml`), their saved keys and the models they
//! offer, as the app sees them. Settings connects providers and picks the
//! agent's model through here; every workspace observes [`Providers`], so a
//! change applies to the next conversation.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

use gpui_kit::{App, BorrowAppContext as _, Global, Task};
use jig_ai::agent::opencode;
use jig_ai::{
    AgentModel, Config, ConfigFile, KeySource, KeyStore, ProviderConfig, ProviderTemplate,
};

use crate::store::persist::providers as persist;

/// How long typing in a key field waits before asking for models, so a
/// key typed by hand isn't tried at every keystroke.
const TYPING_PAUSE: Duration = Duration::from_millis(600);

/// The models a provider offers, as far as Jig knows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ModelList {
    Loading,
    Loaded(Vec<String>),
    Failed(String),
}

pub struct Providers {
    /// `None` without a home folder: the built-in providers, in memory.
    path: Option<PathBuf>,
    /// Where keys typed into Settings go. `None` in tests, so they never
    /// touch the real keychain.
    store: Option<KeyStore>,
    config: Result<Config, String>,
    /// By provider name.
    models: HashMap<String, ModelList>,
    /// Model lists being fetched; replacing one cancels it.
    loads: HashMap<String, Task<()>>,
}

impl Global for Providers {}

impl Providers {
    fn new(path: Option<PathBuf>, store: Option<KeyStore>) -> Self {
        Self {
            config: persist::read(path.as_deref()),
            path,
            store,
            models: HashMap::new(),
            loads: HashMap::new(),
        }
    }
}

pub fn init(cx: &mut App) {
    let (path, store) = persist::user_files();
    cx.set_global(Providers::new(path, store));
    // So ⌘K and the agent's header can list them before Settings opens.
    load_all_models(true, cx);
}

/// Read the file again, after it was edited by hand.
pub fn reload(cx: &mut App) {
    if cx.has_global::<Providers>() {
        cx.update_global::<Providers, _>(|providers, _| {
            providers.config = persist::read(providers.path.as_deref());
        });
    }
}

/// The providers and the agent's model, or why the file couldn't be read.
pub fn config(cx: &App) -> Result<Config, String> {
    match cx.try_global::<Providers>() {
        Some(providers) => providers.config.clone(),
        None => Ok(Config::built_in()),
    }
}

fn store(cx: &App) -> Option<KeyStore> {
    cx.try_global::<Providers>()?.store.clone()
}

/// The key `provider` would send, and where it comes from.
pub fn api_key(provider: &ProviderConfig, cx: &App) -> Option<(String, KeySource)> {
    provider.api_key(store(cx).as_ref(), |var| std::env::var(var).ok())
}

/// Whether `provider` has what it needs to answer: a key, unless it's a
/// local server.
pub fn is_ready(provider: &ProviderConfig, cx: &App) -> bool {
    !provider.needs_key() || api_key(provider, cx).is_some()
}

/// The models `provider` offers, once asked.
pub fn models(provider: &str, cx: &App) -> Option<ModelList> {
    cx.try_global::<Providers>()?.models.get(provider).cloned()
}

/// Every model the connected providers offer, as `provider/model` and a
/// label like "Anthropic · claude-sonnet-5-5".
pub fn offered(cx: &App) -> Vec<(String, String)> {
    offered_by_provider(cx)
        .into_iter()
        .map(|(id, provider, model)| (id, format!("{provider} · {model}")))
        .collect()
}

/// Every model the connected providers offer, as `provider/model`, the
/// provider as Settings names it, and the model.
pub fn offered_by_provider(cx: &App) -> Vec<(String, String, String)> {
    let Ok(config) = config(cx) else {
        return Vec::new();
    };
    let mut offered = Vec::new();
    for provider in &config.providers {
        if let Some(ModelList::Loaded(models)) = models(&provider.name, cx) {
            let label = jig_ai::template_named(&provider.name)
                .map_or(provider.name.as_str(), |template| template.label);
            for model in models {
                offered.push((
                    format!("{}/{model}", provider.name),
                    label.to_string(),
                    model,
                ));
            }
        }
    }
    offered
}

/// OpenCode on `model` (`provider/model`), or on the agent model from the
/// settings.
pub fn agent_target_for(model: Option<&str>, cx: &App) -> Result<opencode::Target, String> {
    let config = config(cx)?;
    let agent = match model {
        Some(id) => {
            let provider = config.resolve(id).map_err(|error| format!("{error:#}"))?;
            let model = provider.model.clone();
            AgentModel::Jig { provider, model }
        }
        None => config.agent().map_err(|error| format!("{error:#}"))?,
    };
    let key = match &agent {
        AgentModel::Jig { provider, .. } => {
            let key = api_key(provider, cx).map(|(key, _)| key);
            if key.is_none() && provider.needs_key() {
                return Err(format!(
                    "\"{}\" has no API key. Add one in Settings > Providers.",
                    provider.name
                ));
            }
            key
        }
        AgentModel::OpenCode(_) => None,
    };
    Ok(opencode::Target::new(&agent, key.as_deref()))
}

/// Change the providers file and tell everyone.
pub fn edit(
    cx: &mut App,
    change: impl FnOnce(&mut ConfigFile) -> anyhow::Result<()>,
) -> Result<(), String> {
    let Some(providers) = cx.try_global::<Providers>() else {
        return Err("Providers aren't loaded.".into());
    };
    let Some(path) = providers.path.clone() else {
        return Err("Couldn't find your home folder.".into());
    };
    let config = persist::edit(&path, change)?;
    cx.update_global::<Providers, _>(|providers, _| providers.config = Ok(config));
    Ok(())
}

/// Set the agent's model, as `provider/model`.
pub fn set_agent(id: &str, cx: &mut App) -> Result<(), String> {
    edit(cx, |file| file.set_agent(id))
}

/// Connect `template`'s provider, saving `key` for it when one is given,
/// and ask it for its models.
pub fn connect(template: &ProviderTemplate, key: Option<&str>, cx: &mut App) -> Result<(), String> {
    edit(cx, |file| file.connect(template))?;
    let pause = match key {
        Some(key) => {
            save_key(template.name, key, cx)?;
            TYPING_PAUSE
        }
        None => Duration::ZERO,
    };
    load_models(template.name, pause, cx);
    Ok(())
}

/// Remove a provider, forgetting its key and models.
pub fn disconnect(provider: &str, cx: &mut App) -> Result<(), String> {
    edit(cx, |file| file.remove(provider))?;
    save_key(provider, "", cx)?;
    cx.update_global::<Providers, _>(|providers, _| {
        providers.models.remove(provider);
        providers.loads.remove(provider);
    });
    Ok(())
}

/// Save `key` for `provider`; an empty one removes it.
fn save_key(provider: &str, key: &str, cx: &mut App) -> Result<(), String> {
    let Some(store) = store(cx) else {
        return Ok(());
    };
    persist::save_key(&store, provider, key)?;
    // Nothing in the file changed, but the agent must be set up again.
    cx.update_global::<Providers, _>(|_, _| {});
    Ok(())
}

/// Ask every provider that's ready for its models: all of them, or with
/// `missing_only` those not asked yet.
pub fn load_all_models(missing_only: bool, cx: &mut App) {
    let Ok(config) = config(cx) else {
        return;
    };
    for provider in &config.providers {
        let asked = models(&provider.name, cx).is_some();
        if is_ready(provider, cx) && !(missing_only && asked) {
            load_models(&provider.name, Duration::ZERO, cx);
        }
    }
}

/// Jig ships with no provider, so the first one connected starts the agent
/// on one of its fast models, once it has listed them. Never over a model
/// already picked.
fn start_on_a_model(name: &str, cx: &mut App) {
    let Ok(config) = config(cx) else {
        return;
    };
    if config.agent.as_ref().is_some_and(|id| !id.is_empty()) {
        return;
    }
    let Some(ModelList::Loaded(models)) = models(name, cx) else {
        return;
    };
    let model = match jig_ai::template_named(name) {
        Some(template) => template.suggested_model(&models),
        None => models.first().cloned(),
    };
    if let Some(model) = model {
        let _ = set_agent(&format!("{name}/{model}"), cx);
    }
}

/// Fetch `name`'s model list after `pause`, replacing any fetch underway.
fn load_models(name: &str, pause: Duration, cx: &mut App) {
    if !cx.has_global::<Providers>() {
        return;
    }
    let Some(provider) = config(cx)
        .ok()
        .and_then(|config| config.provider(name).cloned())
    else {
        return;
    };
    let key = api_key(&provider, cx).map(|(key, _)| key);
    let name = name.to_string();
    let task = cx.spawn({
        let name = name.clone();
        async move |cx| {
            if !pause.is_zero() {
                cx.background_executor().timer(pause).await;
            }
            let result = cx
                .background_executor()
                .spawn(async move { persist::list_models(&provider, key.as_deref()) })
                .await;
            let list = match result {
                Ok(models) if models.is_empty() => ModelList::Failed("It listed no models.".into()),
                Ok(models) => ModelList::Loaded(models),
                Err(error) => ModelList::Failed(format!("{error:#}")),
            };
            cx.update_global::<Providers, _>(|providers, _| {
                providers.models.insert(name.clone(), list);
                providers.loads.remove(&name);
            });
            cx.update(|cx| start_on_a_model(&name, cx));
        }
    });
    cx.update_global::<Providers, _>(|providers, _| {
        providers.models.insert(name.clone(), ModelList::Loading);
        providers.loads.insert(name, task);
    });
}

/// Keep providers in `dir` instead of the user's config, for pictures of
/// Settings.
#[cfg(feature = "snapshots")]
pub fn init_at(dir: &std::path::Path, cx: &mut App) {
    let (path, store) = persist::files_in(dir);
    cx.set_global(Providers::new(path, store));
}

/// Providers in a temporary folder, with keys in a file there.
#[cfg(any(test, feature = "testing"))]
pub fn init_temp(cx: &mut gpui_kit::TestAppContext) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let (path, store) = persist::files_in(dir.path());
    cx.update(|cx| cx.set_global(Providers::new(path, store)));
    dir
}

#[cfg(test)]
mod tests;
