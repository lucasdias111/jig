//! Format Document (⇧⌥F), and what saving does to a file when Settings
//! asks: format it, trim trailing whitespace, end it with a newline.
//!
//! The file's language server formats when it can (only the selection, if
//! there is one and the server formats ranges); otherwise the first
//! installed formatter for the language in [`crate::formatters`] gets the
//! text on stdin. Either way the result lands in the buffer as one undo
//! step, only the lines that changed are touched, and the cursor stays
//! where it was in the code. Nothing is formatted while a change is under
//! review.

use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use futures::FutureExt as _;
use futures::channel::oneshot;
use futures::future::{Either, LocalBoxFuture, select};
use gpui_kit::*;
use jig_editor::EditorHandle;
use serde_json::{Value, json};

use super::{EditFormatters, FormatDocument, Workspace};
use crate::formatters::{self, Formatter};
use crate::lsp::{Client, Encoding};
use crate::{languages, settings, text_edits};

/// How long Format Document waits.
const FORMAT_TIMEOUT: Duration = Duration::from_secs(10);
/// How long saving waits for the formatter before saving the file as it is.
pub(super) const SAVE_TIMEOUT: Duration = Duration::from_secs(2);
/// How long a note stays in the status bar.
const STATUS_TIMEOUT: Duration = Duration::from_secs(8);

/// The file's formatted text, or why there isn't any.
type Job = LocalBoxFuture<'static, Result<String, String>>;

/// A save waiting on its formatter.
pub(super) struct PendingSave {
    tab: EntityId,
    _task: Task<()>,
}

impl Workspace {
    pub(super) fn format_document(
        &mut self,
        _: &FormatDocument,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.home || self.modal_open() {
            return;
        }
        if self.previewing() {
            self.show_note("Accept or reject the change first.".into(), window, cx);
            return;
        }
        let tab = self.tab().id();
        let text = self.editor().text(cx);
        let selection = self.editor().selection(cx);
        let range = (!selection.is_empty()).then(|| selection.clone());
        let job = match self.format_job(self.active, &text, range, FORMAT_TIMEOUT) {
            Ok(job) => job,
            Err(reason) => {
                self.show_note(reason, window, cx);
                return;
            }
        };
        cx.spawn_in(window, async move |this, cx| {
            let timer = cx.background_executor().timer(FORMAT_TIMEOUT);
            let result = match select(job, Box::pin(timer)).await {
                Either::Left((result, _)) => result,
                Either::Right(_) => Err("The formatter didn't answer in time.".into()),
            };
            this.update_in(cx, |this, window, cx| {
                // Typed into, or a change arrived: the answer is about
                // other text.
                if this.home
                    || this.tab().id() != tab
                    || this.previewing()
                    || this.editor().text(cx) != text
                {
                    return;
                }
                match result {
                    Ok(formatted) if formatted == text => {
                        this.show_status("Already formatted.".into(), cx)
                    }
                    Ok(formatted) => {
                        let ix = this.active;
                        this.put_formatted(ix, &text, &formatted, window, cx);
                    }
                    Err(reason) => this.show_note(reason, window, cx),
                }
            })
            .ok();
        })
        .detach();
    }

    /// Save the current tab to `path`, formatting and tidying it first as
    /// Settings says. A formatter that takes too long is given up on, and
    /// the file saved as it is.
    pub(super) fn save_tidied(
        &mut self,
        path: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let tab = self.tab().id();
        if self
            .pending_save
            .as_ref()
            .is_some_and(|save| save.tab == tab)
        {
            return;
        }
        // Formatting would change the change under review.
        if self.previewing() {
            self.save_to(&path, window, cx);
            return;
        }
        let language = languages::language_for(&path, &self.settings.languages);
        if !settings::get(cx).formatting.on_save(language) {
            let text = self.editor().text(cx);
            self.save_formatted(tab, path, text, Ok(None), window, cx);
            return;
        }
        let text = self.editor().text(cx);
        let job = match self.format_job(self.active, &text, None, SAVE_TIMEOUT) {
            Ok(job) => job,
            Err(reason) => {
                self.save_formatted(tab, path, text, Err(reason), window, cx);
                return;
            }
        };
        let task = cx.spawn_in(window, async move |this, cx| {
            let timer = cx.background_executor().timer(SAVE_TIMEOUT);
            let result = match select(job, Box::pin(timer)).await {
                Either::Left((result, _)) => result.map(Some),
                Either::Right(_) => Err(format!(
                    "The formatter took more than {} s.",
                    SAVE_TIMEOUT.as_secs()
                )),
            };
            this.update_in(cx, |this, window, cx| {
                this.pending_save = None;
                this.save_formatted(tab, path, text, result, window, cx);
            })
            .ok();
        });
        self.pending_save = Some(PendingSave { tab, _task: task });
    }

    /// Finish saving tab `tab`, which read `text` when saving began, with
    /// what its formatter gave: new text, nothing to do, or why not.
    fn save_formatted(
        &mut self,
        tab: EntityId,
        path: PathBuf,
        text: String,
        formatted: Result<Option<String>, String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(ix) = self.tabs.iter().position(|t| t.id() == tab) else {
            return;
        };
        let current = self.tabs[ix].editor.text(cx);
        let (new, note) = if self.previewing() {
            (None, Some("A change is under review.".to_string()))
        } else if current != text {
            (None, Some("The file changed while formatting.".to_string()))
        } else {
            let formatting = settings::get(cx).formatting;
            let tidy = |text: &str| {
                formatters::tidy(
                    text,
                    formatting.trim_trailing_whitespace,
                    formatting.final_newline,
                )
            };
            match formatted {
                Ok(new) => (Some(tidy(new.as_deref().unwrap_or(&text))), None),
                Err(reason) => (Some(tidy(&text)), Some(reason)),
            }
        };
        if let Some(new) = new.filter(|new| *new != current) {
            self.put_formatted(ix, &current, &new, window, cx);
        }
        if ix == self.active {
            self.save_to(&path, window, cx);
        } else {
            self.save_in_background(ix, &path, window, cx);
        }
        if let Some(note) = note {
            self.show_status(format!("Saved without formatting. {note}").into(), cx);
        }
    }

    /// Save a tab that isn't the one showing, as Save would have had it
    /// still been.
    fn save_in_background(
        &mut self,
        ix: usize,
        path: &Path,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let text = self.tabs[ix].editor.text(cx);
        if let Err(error) = self.tabs[ix].document.save(path, &text) {
            self.show_error(&format!("Could not save: {error:#}"), window, cx);
            return;
        }
        self.tabs[ix].dirty = false;
        if let Some(lsp) = &self.tabs[ix].lsp {
            lsp.client.save(&lsp.uri);
        }
        self.update_title(window);
        cx.notify();
    }

    /// Replace tab `ix`'s `old` text with `new`: one undo step touching
    /// only the lines that differ, the selection carried along.
    fn put_formatted(
        &mut self,
        ix: usize,
        old: &str,
        new: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let edits = text_edits::line_edits(old, new);
        let Some((span, replacement)) = text_edits::splice(old, &edits) else {
            return;
        };
        let editor = self.tabs[ix].editor.clone();
        let selection = editor.selection(cx);
        editor.apply_edit(span, &replacement, window, cx);
        let start = text_edits::carry(old, selection.start, &edits);
        let end = text_edits::carry(old, selection.end, &edits);
        editor.select(start..end, cx);
    }

    /// What formats tab `ix`'s `text` (or `range` of it), started; or why
    /// nothing can.
    fn format_job(
        &self,
        ix: usize,
        text: &str,
        range: Option<Range<usize>>,
        limit: Duration,
    ) -> Result<Job, String> {
        let tab = &self.tabs[ix];
        let Some(path) = tab.document.path.clone() else {
            return Err("Save the file first, so Jig knows how to format it.".into());
        };
        let language = tab.document.language(&self.settings.languages);
        let registry = formatters::registry();
        let external = registry.for_language(language, &path);
        let server = tab.lsp.as_ref().and_then(|lsp| {
            let can = lsp.client.formatting();
            let range = range.clone().filter(|_| can.range);
            (can.document || range.is_some()).then(|| (lsp.client.clone(), lsp.uri.clone(), range))
        });
        let text = text.to_string();
        let external_job = external.clone().map(|formatter| {
            let root = crate::project::root_for(&path);
            let (text, path) = (text.clone(), path.clone());
            move || run_external(formatter, text, path, root, limit)
        });
        if let Some((client, uri, range)) = server {
            let indentation = tab.indentation;
            let options = json!({
                "tabSize": indentation.tab_size,
                "insertSpaces": !indentation.hard_tabs,
            });
            let request = server_request(&client, &uri, &text, range, options);
            let encoding = client.encoding();
            return Ok(async move {
                match request.await {
                    Ok(Ok(edits)) => server_text(&text, &edits, encoding),
                    // The server couldn't: the formatter, if there is one.
                    failed => match external_job {
                        Some(job) => job().await,
                        None => Err(match failed {
                            Ok(Err(error)) => {
                                format!("The language server couldn't format: {error}")
                            }
                            _ => "The language server stopped.".into(),
                        }),
                    },
                }
            }
            .boxed_local());
        }
        if let Some(job) = external_job {
            return Ok(job());
        }
        let label = languages::written_in(&path, &self.settings.languages)
            .map_or("this file", |language| language.label);
        Err(match registry.knows(language) {
            Some(formatter) => format!(
                "Nothing formats {label}: {} isn't installed.",
                formatter.name
            ),
            None => format!("Nothing formats {label}. Add a formatter in formatters.toml."),
        })
    }

    /// Show `message` in the status bar for a while.
    pub(super) fn show_status(&mut self, message: SharedString, cx: &mut Context<Self>) {
        self.next_run_id += 1;
        let id = self.next_run_id;
        let task = cx.spawn(async move |this, cx| {
            cx.background_executor().timer(STATUS_TIMEOUT).await;
            this.update(cx, |this, cx| {
                if this.status_note.as_ref().is_some_and(|note| note.id == id) {
                    this.status_note = None;
                    cx.notify();
                }
            })
            .ok();
        });
        self.status_note = Some(StatusNote {
            id,
            message,
            _task: task,
        });
        cx.notify();
    }

    /// Open `~/.config/jig/formatters.toml`, starting it from the template.
    pub(super) fn edit_formatters(
        &mut self,
        _: &EditFormatters,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(path) = formatters::user_path() else {
            return;
        };
        if !path.exists() {
            let created = path
                .parent()
                .map_or(Ok(()), std::fs::create_dir_all)
                .and_then(|()| std::fs::write(&path, formatters::USER_TEMPLATE));
            if let Err(error) = created {
                self.show_error(
                    &format!("Couldn't create {}: {error}", path.display()),
                    window,
                    cx,
                );
                return;
            }
        }
        self.open_file(&path, window, cx);
    }
}

/// A message in the status bar, gone after a while.
pub(super) struct StatusNote {
    id: u64,
    pub(super) message: SharedString,
    _task: Task<()>,
}

/// Ask the server to format `text`, or `range` of it.
fn server_request(
    client: &Arc<Client>,
    uri: &str,
    text: &str,
    range: Option<Range<usize>>,
    options: Value,
) -> oneshot::Receiver<crate::lsp::Response> {
    let encoding = client.encoding();
    let mut params = json!({"textDocument": {"uri": uri}, "options": options});
    // The server formats what it was last sent.
    client.change(uri, text);
    match range {
        Some(range) => {
            params["range"] = json!(lsp_types::Range::new(
                crate::lsp::position(text, range.start, encoding),
                crate::lsp::position(text, range.end, encoding),
            ));
            client.request("textDocument/rangeFormatting", params)
        }
        None => client.request("textDocument/formatting", params),
    }
}

/// `text` with the server's `TextEdit[]` made; `null` is no change.
fn server_text(text: &str, edits: &Value, encoding: Encoding) -> Result<String, String> {
    let edits = text_edits::from_lsp(text, edits, encoding);
    text_edits::applied(text, &edits)
        .ok_or_else(|| "The language server's edits overlap, so none were made.".into())
}

/// Run `formatter` on a thread of its own, so the window doesn't wait.
fn run_external(
    formatter: Arc<Formatter>,
    text: String,
    path: PathBuf,
    root: PathBuf,
    limit: Duration,
) -> Job {
    let (tx, rx) = oneshot::channel();
    std::thread::spawn(move || {
        let _ = tx.send(formatter.format(&text, &path, &root, limit));
    });
    async move {
        rx.await
            .unwrap_or_else(|_| Err("The formatter stopped.".into()))
    }
    .boxed_local()
}
