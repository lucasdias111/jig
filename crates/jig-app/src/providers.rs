//! The AI providers (`config.toml`), their saved keys and the models they
//! offer, as the app sees them. Settings connects providers and picks each
//! lane's model through here; every workspace observes [`Providers`], so a
//! change applies to the next command.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use gpui_kit::{App, BorrowAppContext as _, Global, Task};
use jig_ai::agent::AgentTarget;
use jig_ai::{
    AgentModel, Config, ConfigFile, KeySource, KeyStore, Provider, ProviderConfig, ProviderTemplate,
};

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
            config: read(path.as_ref()),
            path,
            store,
            models: HashMap::new(),
            loads: HashMap::new(),
        }
    }
}

fn read(path: Option<&PathBuf>) -> Result<Config, String> {
    Config::load(path.map(PathBuf::as_path)).map_err(|error| format!("{error:#}"))
}

pub fn init(cx: &mut App) {
    cx.set_global(Providers::new(Config::user_path(), KeyStore::system()));
}

/// Read the file again, after it was edited by hand.
pub fn reload(cx: &mut App) {
    if cx.has_global::<Providers>() {
        cx.update_global::<Providers, _>(|providers, _| {
            providers.config = read(providers.path.as_ref());
        });
    }
}

/// The providers and lanes, or why the file couldn't be read.
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

/// The provider quick commands call, with its model.
pub fn quick(cx: &App) -> Result<ProviderConfig, String> {
    config(cx)?.quick().map_err(|error| format!("{error:#}"))
}

/// The quick model, ready to call.
pub fn build(cx: &App) -> Result<Arc<dyn Provider>, String> {
    let provider = quick(cx)?;
    provider
        .build_with(api_key(&provider, cx).map(|(key, _)| key))
        .map_err(|error| format!("{error:#}"))
}

/// The agent's model and what OpenCode needs to reach it.
pub fn agent_target(cx: &App) -> Result<AgentTarget, String> {
    let agent = config(cx)?.agent().map_err(|error| format!("{error:#}"))?;
    let key = match &agent {
        AgentModel::Jig { provider, .. } => {
            let key = api_key(provider, cx).map(|(key, _)| key);
            if key.is_none() && provider.needs_key() {
                return Err(format!(
                    "\"{}\" has no API key. Add one in Settings > Models.",
                    provider.name
                ));
            }
            key
        }
        AgentModel::OpenCode(_) => None,
    };
    Ok(AgentTarget::new(&agent, key.as_deref()))
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
    let config = ConfigFile::open(&path)
        .and_then(|mut file| {
            change(&mut file)?;
            file.save()
        })
        .map_err(|error| format!("{error:#}"))?;
    cx.update_global::<Providers, _>(|providers, _| providers.config = Ok(config));
    Ok(())
}

/// Set the quick model, as `provider/model`.
pub fn set_quick(id: &str, cx: &mut App) -> Result<(), String> {
    edit(cx, |file| file.set_quick(id))
}

/// Set the agent model; `None` follows quick commands.
pub fn set_agent(id: Option<&str>, cx: &mut App) -> Result<(), String> {
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
    let result = if key.trim().is_empty() {
        store.delete(provider)
    } else {
        store.set(provider, key).map(drop)
    };
    result.map_err(|error| format!("{error:#}"))?;
    // Nothing in the file changed, but the quick model must be built again.
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
                .spawn(async move { provider.list_models(key.as_deref()) })
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
    let store = KeyStore::File(dir.join("keys.toml"));
    cx.set_global(Providers::new(Some(dir.join("config.toml")), Some(store)));
}

/// Providers in a temporary folder, with keys in a file there.
#[cfg(test)]
pub fn init_temp(cx: &mut gpui_kit::TestAppContext) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    let store = KeyStore::File(dir.path().join("keys.toml"));
    cx.update(|cx| cx.set_global(Providers::new(Some(path), Some(store))));
    dir
}

#[cfg(test)]
mod tests {
    use gpui_kit::TestAppContext;
    use jig_ai::template_named;

    use super::*;

    #[gpui_kit::test]
    fn connecting_with_a_key_makes_the_provider_usable(cx: &mut TestAppContext) {
        let dir = init_temp(cx);
        let openai = template_named("openai").unwrap();
        cx.update(|cx| {
            if std::env::var("OPENAI_API_KEY").is_err() {
                connect(openai, None, cx).unwrap();
                let provider = config(cx).unwrap().provider("openai").cloned().unwrap();
                assert!(!is_ready(&provider, cx), "no key yet");
            }
            connect(openai, Some("sk-test"), cx).unwrap();
            let provider = config(cx).unwrap().provider("openai").cloned().unwrap();
            assert_eq!(
                api_key(&provider, cx),
                Some(("sk-test".into(), KeySource::File))
            );
            assert_eq!(models("openai", cx), Some(ModelList::Loading));

            set_quick("openai/gpt-test", cx).unwrap();
            assert_eq!(quick(cx).unwrap().model, "gpt-test");
            assert!(build(cx).is_ok());
        });
        let file = std::fs::read_to_string(dir.path().join("config.toml")).unwrap();
        assert!(file.contains("quick = \"openai/gpt-test\""));

        cx.update(|cx| {
            disconnect("openai", cx).unwrap();
            assert!(quick(cx).is_err(), "its lane needs a new model");
            assert_eq!(models("openai", cx), None);
        });
        let keys = std::fs::read_to_string(dir.path().join("keys.toml")).unwrap();
        assert!(!keys.contains("sk-test"), "disconnecting forgets the key");
    }

    #[gpui_kit::test]
    fn the_agent_gets_its_provider_handed_to_opencode(cx: &mut TestAppContext) {
        let _dir = init_temp(cx);
        cx.update(|cx| {
            connect(template_named("ollama").unwrap(), None, cx).unwrap();
            set_quick("ollama/qwen3:8b", cx).unwrap();
            let target = agent_target(cx).unwrap();
            assert_eq!(target.model, "jig-ollama/qwen3:8b");
            assert!(target.config.unwrap()["provider"]["jig-ollama"].is_object());

            set_agent(Some("opencode-go/glm-5.3"), cx).unwrap();
            if std::env::var("OPENCODE_API_KEY").is_err() {
                assert!(agent_target(cx).unwrap_err().contains("has no API key"));
            }
        });
    }
}
