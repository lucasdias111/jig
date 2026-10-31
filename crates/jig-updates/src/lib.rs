//! Updating Jig from its GitHub releases. Jig looks for a newer release
//! shortly after it starts and once a day (Settings > Updates turns that
//! off), downloads the build for this computer and checks it was signed
//! with Jig's key. Nothing is installed until you choose Restart to Update,
//! in the status bar or when Check for Updates… finds it.

mod install;
mod release;

use std::path::PathBuf;
use std::time::Duration;

use gpui_kit::*;

pub use release::Version;

actions!(jig, [CheckForUpdates, RestartToUpdate]);

/// Long enough after starting not to slow it down.
const FIRST_CHECK: Duration = Duration::from_secs(10);
const CHECK_EVERY: Duration = Duration::from_secs(24 * 60 * 60);

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum State {
    Idle,
    Checking,
    UpToDate,
    Downloading(Version),
    /// Downloaded and its signature checked, waiting for a restart.
    Ready {
        version: Version,
        archive: PathBuf,
    },
    /// Newer, but this copy of Jig can't put it in place itself.
    Manual {
        version: Version,
        reason: String,
    },
    Failed(String),
}

pub struct Updates {
    state: State,
    /// Say how the check underway went, as Check for Updates… asked.
    announce: bool,
    check: Option<Task<()>>,
    _schedule: Option<Task<()>>,
}

impl Global for Updates {}

/// Start looking for updates, shortly and then daily, whenever `automatic`
/// says to (the app reads it from Settings > Updates).
pub fn init(automatic: fn(&App) -> bool, cx: &mut App) {
    install::clean_up();
    let schedule = cx.spawn(async move |cx| {
        cx.background_executor().timer(FIRST_CHECK).await;
        loop {
            cx.update(|cx| {
                if automatic(cx) {
                    check(false, cx);
                }
            });
            cx.background_executor().timer(CHECK_EVERY).await;
        }
    });
    cx.set_global(Updates {
        state: State::Idle,
        announce: false,
        check: None,
        _schedule: Some(schedule),
    });
    cx.on_action(|_: &CheckForUpdates, cx| check(true, cx));
}

/// Where updates stand; `Idle` until `init` has run, as in tests.
pub fn state(cx: &App) -> State {
    cx.try_global::<Updates>()
        .map_or(State::Idle, |updates| updates.state.clone())
}

/// The version running, for Settings and About.
pub fn current_version() -> String {
    Version::current().to_string()
}

fn set_state(state: State, cx: &mut App) {
    if cx.has_global::<Updates>() {
        cx.update_global::<Updates, _>(|updates, _| updates.state = state);
    }
}

/// Look for a newer release, and download it if Jig can install it.
/// `manual` says how it went in a dialog, as Check for Updates… does;
/// otherwise only a found update shows, in the status bar.
pub fn check(manual: bool, cx: &mut App) {
    if !cx.has_global::<Updates>() {
        return;
    }
    let updates = cx.global::<Updates>();
    match updates.state {
        State::Ready { .. } => {
            if manual {
                announce(cx);
            }
            return;
        }
        State::Checking | State::Downloading(_) => {
            if manual {
                cx.update_global::<Updates, _>(|updates, _| updates.announce = true);
            }
            return;
        }
        _ => {}
    }
    let installation = install::installation();
    // A build from source never looks unless asked.
    if !manual && installation.is_err() {
        return;
    }
    let task = cx.spawn(async move |cx| {
        let found = cx
            .background_executor()
            .spawn(async { release::newer_than(Version::current()) })
            .await;
        let state = match (found, installation) {
            (Err(error), _) => State::Failed(format!("{error:#}")),
            (Ok(None), _) => State::UpToDate,
            (Ok(Some(release)), Err(reason)) => State::Manual {
                version: release.version,
                reason,
            },
            (Ok(Some(release)), Ok(_)) => {
                cx.update(|cx| set_state(State::Downloading(release.version), cx));
                let dir = std::env::temp_dir().join("jig-update");
                let version = release.version;
                let downloaded = cx
                    .background_executor()
                    .spawn(async move { release::download(&release, &dir) })
                    .await;
                match downloaded {
                    Ok(archive) => State::Ready { version, archive },
                    Err(error) => State::Failed(format!("{error:#}")),
                }
            }
        };
        cx.update(|cx| {
            set_state(state, cx);
            let announce_it = cx.update_global::<Updates, _>(|updates, _| {
                updates.check = None;
                std::mem::take(&mut updates.announce)
            });
            if announce_it {
                announce(cx);
            }
        });
    });
    cx.update_global::<Updates, _>(|updates, _| {
        updates.state = State::Checking;
        updates.announce = manual;
        updates.check = Some(task);
    });
}

/// Say where updates stand, in a dialog over the frontmost window.
fn announce(cx: &mut App) {
    let Some(window) = cx.active_window().or_else(|| cx.windows().first().copied()) else {
        return;
    };
    let current = Version::current();
    let (level, title, detail, answers): (_, String, String, &[&str]) = match state(cx) {
        State::UpToDate => (
            PromptLevel::Info,
            "Jig is up to date".into(),
            format!("Version {current} is the latest."),
            &["OK"],
        ),
        State::Ready { version, .. } => (
            PromptLevel::Info,
            format!("Jig {version} is ready"),
            format!(
                "You have {current}. Jig restarts to update, asking about unsaved changes first."
            ),
            &["Restart to Update", "Later"],
        ),
        State::Manual { version, reason } => (
            PromptLevel::Info,
            format!("Jig {version} is available"),
            reason,
            &["Download", "Later"],
        ),
        State::Failed(error) => (
            PromptLevel::Warning,
            "Couldn't check for updates".into(),
            error,
            &["OK"],
        ),
        State::Idle | State::Checking | State::Downloading(_) => return,
    };
    window
        .update(cx, |_, window, cx| {
            let answer = window.prompt(level, &title, Some(&detail), answers, cx);
            cx.spawn(async move |cx| {
                if answer.await != Ok(0) {
                    return;
                }
                cx.update(|cx| match state(cx) {
                    State::Ready { .. } => {
                        jig_commands::windows::send_to_editor(Box::new(RestartToUpdate), cx)
                    }
                    State::Manual { .. } => open_releases_page(cx),
                    _ => {}
                });
            })
            .detach();
        })
        .ok();
}

pub fn open_releases_page(cx: &mut App) {
    cx.open_url(&release::releases_page());
}

/// Put the downloaded update in place and start it on `reopen`, then quit.
/// The caller has already asked about unsaved changes.
pub fn restart(reopen: Option<PathBuf>, cx: &mut App) -> Result<(), String> {
    let State::Ready { version, archive } = state(cx) else {
        return Ok(());
    };
    let installed = install::installation()
        .map_err(anyhow::Error::msg)
        .and_then(|installation| install::install(&installation, &archive))
        .and_then(|installed| {
            install::relaunch(&installed, reopen.as_deref())?;
            Ok(installed)
        });
    match installed {
        Ok(_) => {
            let _ = std::fs::remove_file(&archive);
            cx.quit();
            Ok(())
        }
        Err(error) => {
            let reason = format!("{error:#}");
            set_state(
                State::Manual {
                    version,
                    reason: reason.clone(),
                },
                cx,
            );
            Err(reason)
        }
    }
}

/// Put updates in `state`, as a check would.
#[cfg(any(test, feature = "testing"))]
pub fn set_for_test(state: State, cx: &mut App) {
    cx.set_global(Updates {
        state,
        announce: false,
        check: None,
        _schedule: None,
    });
}
