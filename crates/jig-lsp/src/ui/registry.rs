//! The running servers, one per server and root, shared by every window
//! as a GPUI global, and the channels their threads report on.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use gpui_kit::{App, Global};
use jig_languages::Server;

use super::diagnostics;
use crate::domain::diagnostics::Published;
use crate::store::client::Client;
use crate::store::servers::{command_for, init_options, root_for, server_for};

/// A server and the root it runs in.
type Key = (&'static str, PathBuf);

/// The running servers, one per server and root.
#[derive(Default)]
pub struct Registry {
    clients: HashMap<Key, Arc<Client>>,
    /// Servers started in a folder without their markers, as when a file
    /// opens in a project before `cargo init` has run. rust-analyzer never
    /// finds a project that appears after it starts, so these are started
    /// again once their markers show up ([`Registry::outgrown`]).
    unmarked: HashSet<Key>,
    /// Where every server's problems go, on their way to
    /// [`diagnostics::Diagnostics`].
    diagnostics: Option<futures::channel::mpsc::UnboundedSender<Published>>,
    /// Where servers say they stopped before they started, on their way to
    /// [`Failures`].
    failures: Option<futures::channel::mpsc::UnboundedSender<()>>,
}

impl Global for Registry {}

impl Registry {
    /// Whether the server at `key` started without its markers and its root
    /// has them now.
    fn outgrown(&self, server: &Server, key: &Key) -> bool {
        self.unmarked.contains(key) && server.marks(&key.1)
    }

    fn insert(&mut self, server: &Server, key: Key, client: Arc<Client>) {
        if server.marks(&key.1) {
            self.unmarked.remove(&key);
        } else {
            self.unmarked.insert(key.clone());
        }
        self.clients.insert(key, client);
    }
}

/// Bumped when a server stops before it has started, for workspaces to say
/// why ([`Client::failure`]) rather than quietly go without it.
#[derive(Default)]
pub struct Failures(pub u64);

impl Global for Failures {}

/// Use `client` for `server`'s files under `root`, for tests.
#[cfg(any(test, feature = "testing"))]
pub fn register(server: &'static str, root: PathBuf, client: Arc<Client>, cx: &mut App) {
    client.on_diagnostics(diagnostics_sink(cx));
    client.on_failure(failures_sink(cx));
    let server = jig_languages::server_named(server).expect("a known server");
    cx.default_global::<Registry>()
        .insert(server, (&server.name, root), client);
}

/// The channel servers' threads send problems down, read on the main thread
/// into [`diagnostics::Diagnostics`].
fn diagnostics_sink(cx: &mut App) -> futures::channel::mpsc::UnboundedSender<Published> {
    if let Some(sink) = &cx.default_global::<Registry>().diagnostics {
        return sink.clone();
    }
    let (sink, mut reports) = futures::channel::mpsc::unbounded();
    cx.spawn(async move |cx| {
        use futures::StreamExt as _;
        while let Some(published) = reports.next().await {
            cx.update(|cx| diagnostics::publish(published, cx));
        }
    })
    .detach();
    cx.default_global::<Registry>().diagnostics = Some(sink.clone());
    sink
}

/// The channel servers' threads say they failed down, read on the main
/// thread into [`Failures`].
fn failures_sink(cx: &mut App) -> futures::channel::mpsc::UnboundedSender<()> {
    if let Some(sink) = &cx.default_global::<Registry>().failures {
        return sink.clone();
    }
    let (sink, mut failures) = futures::channel::mpsc::unbounded();
    cx.spawn(async move |cx| {
        use futures::StreamExt as _;
        while failures.next().await.is_some() {
            cx.update(|cx| cx.default_global::<Failures>().0 += 1);
        }
    })
    .detach();
    cx.default_global::<Registry>().failures = Some(sink.clone());
    sink
}

/// The client for `file`, starting its server if it isn't running. `None`
/// when no server serves `language` or none is installed.
pub fn client_for(file: &Path, language: &str, cx: &mut App) -> Option<Arc<Client>> {
    let server = server_for(language)?;
    let file = std::path::absolute(file).ok()?;
    let root = root_for(server, &file);
    let key = (server.name.as_str(), root.clone());
    let registry = cx.default_global::<Registry>();
    if registry.outgrown(server, &key) {
        // Its tabs let go of it as they move to the new one, and the last
        // to go shuts it down.
        registry.clients.remove(&key);
    } else if let Some(client) = registry.clients.get(&key) {
        return (!client.is_dead()).then(|| client.clone());
    }
    // Tests use fake servers, never what happens to be installed, unless
    // a live test asks for the real ones.
    if cfg!(any(test, feature = "testing")) && std::env::var_os("JIG_LIVE_LSP").is_none() {
        return None;
    }
    // One that isn't installed is kept too, dead, so it isn't looked for
    // again.
    let command_root = root.clone();
    let init_options = init_options(server);
    let client = Client::start(&server.name, root, init_options, move || {
        command_for(server, &command_root)
    });
    registry.insert(server, key, client.clone());
    client.on_diagnostics(diagnostics_sink(cx));
    client.on_failure(failures_sink(cx));
    (!client.is_dead()).then_some(client)
}

/// Whether `client` is still the one [`client_for`] gives `file`: not when
/// a marker has since appeared, giving the file another root or its
/// server's root the markers it started without.
pub fn is_current(client: &Arc<Client>, file: &Path, language: &str, cx: &App) -> bool {
    let Some(server) = server_for(language) else {
        return false;
    };
    let Ok(file) = std::path::absolute(file) else {
        return true;
    };
    let key = (server.name.as_str(), root_for(server, &file));
    cx.try_global::<Registry>().is_some_and(|registry| {
        !registry.outgrown(server, &key)
            && registry
                .clients
                .get(&key)
                .is_some_and(|current| Arc::ptr_eq(current, client))
    })
}

/// Why `file`'s server stopped before it started, if it did.
pub fn failure_for(file: &Path, language: &str, cx: &App) -> Option<String> {
    let server = server_for(language)?;
    let root = root_for(server, &std::path::absolute(file).ok()?);
    cx.try_global::<Registry>()?
        .clients
        .get(&(server.name.as_str(), root))?
        .failure()
}

/// Bumped when a server is installed or uninstalled, for workspaces to
/// open their files in it again.
#[derive(Default)]
pub struct ServersChanged(pub u64);

impl Global for ServersChanged {}

/// A server was installed or uninstalled: forget the ones that weren't
/// found when last looked for, and tell the workspaces.
pub fn servers_changed(cx: &mut App) {
    let Registry {
        clients, unmarked, ..
    } = cx.default_global::<Registry>();
    clients.retain(|_, client| !client.is_dead());
    unmarked.retain(|key| clients.contains_key(key));
    cx.default_global::<ServersChanged>().0 += 1;
}
