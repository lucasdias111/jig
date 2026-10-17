//! Quick Fix (⌘.): ways to fix the problems on the cursor's line, in GPUI
//! Kit's code action list under the cursor. The language server's own fixes
//! come first (`textDocument/codeAction`, quick fixes with their edits),
//! then Fix with Agent, which starts a conversation given the problems, for
//! fixes the server doesn't have, including those that reach past the line,
//! such as a dependency missing from the manifest. The problem panels offer
//! the same, under the message.
//!
//! A server's fix lands in the buffers as rename's edits do: unsaved, one
//! undo step per file. The agent's go through review like any other edit.

use std::ops::Range;
use std::rc::Rc;
use std::time::Duration;

use anyhow::Result;
use gpui_kit::base::input::DiagnosticEntry;
use gpui_kit::component::input::{CodeActionProvider, EditorState};
use gpui_kit::component::{ActiveTheme as _, Icon, h_flex, v_flex};
use gpui_kit::*;
use jig_commands::surface;
use jig_editor::EditorHandle;
use lsp_types::{CodeAction, CodeActionKind, CodeActionOrCommand, Command};
use serde_json::{Value, json};

use super::Workspace;
use super::commands::Ask;
use super::definitions::with_timeout;
use crate::lsp;

/// Past this, the list opens with Fix with Agent alone.
const FIX_TIMEOUT: Duration = Duration::from_secs(3);
/// How many of the server's fixes a problem's panel lists.
const PANEL_FIXES: usize = 3;
const FIX_WITH_AGENT: &str = "jig.fixWithAgent";
const AGENT_FIX: &str = "Fix the problems the language server reports here. If the fix belongs outside this code, such as a dependency missing from the project's manifest, make it there.";

/// Lucide "wrench".
const FIX_ICON: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M14.7 6.3a1 1 0 0 0 0 1.4l1.6 1.6a1 1 0 0 0 1.4 0l3.106-3.105c.32-.322.863-.22.983.218a6 6 0 0 1-8.259 7.057l-7.91 7.91a1 1 0 0 1-2.999-3l7.91-7.91a6 6 0 0 1 7.057-8.259c.438.12.54.662.219.984z"/></svg>"#;

/// Give a tab's editor the Quick Fix list.
pub(super) fn install(state: &Entity<EditorState>, cx: &mut Context<Workspace>) {
    let provider = Rc::new(QuickFixes {
        workspace: cx.entity().downgrade(),
        editor: state.entity_id(),
    });
    state.update(cx, |state, _| {
        state.lsp_mut().code_action_providers = vec![provider];
    });
}

impl Workspace {
    /// Ask the server of the tab of `editor` for its fixes for the problems
    /// on the lines `range` covers. `None` without a server, or without a
    /// problem there it reported.
    pub(super) fn server_fixes(
        &self,
        editor: EntityId,
        range: Range<usize>,
        cx: &App,
    ) -> Option<impl AsyncFnOnce(&AsyncApp) -> Vec<CodeAction> + use<>> {
        let tab = self.tabs.iter().find(|tab| tab.id() == editor)?;
        let tab_lsp = tab.lsp.as_ref()?;
        let text = tab.editor.text(cx);
        let lines = line_span(&text, range);
        let (first, last) = (line_of(&text, lines.start), line_of(&text, lines.end));
        let problems: Vec<Value> = tab
            .problems
            .reported(cx)
            .into_iter()
            .filter(|item| item.range.start.line <= last && item.range.end.line >= first)
            .filter_map(|item| serde_json::to_value(item).ok())
            .collect();
        if problems.is_empty() {
            return None;
        }
        let client = tab_lsp.client.clone();
        client.change(&tab_lsp.uri, &text);
        let encoding = client.encoding();
        let request = client.request(
            "textDocument/codeAction",
            json!({
                "textDocument": {"uri": tab_lsp.uri},
                "range": {
                    "start": lsp::position(&text, lines.start, encoding),
                    "end": lsp::position(&text, lines.end, encoding),
                },
                "context": {"diagnostics": problems, "only": ["quickfix"], "triggerKind": 1},
            }),
        );
        Some(async move |cx: &AsyncApp| {
            with_timeout(request, FIX_TIMEOUT, cx)
                .await
                .map(|answer| quick_fixes(&answer))
                .unwrap_or_default()
        })
    }

    /// Make a server's fix, in the tab of `editor` and any other files it
    /// changes.
    pub(super) fn apply_fix(
        &mut self,
        editor: EntityId,
        action: &CodeAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(edit) = action
            .edit
            .as_ref()
            .and_then(|edit| serde_json::to_value(edit).ok())
        else {
            return;
        };
        if self.previewing() {
            self.show_note(
                "Accept or reject the change under review first.".into(),
                window,
                cx,
            );
            return;
        }
        let Some(encoding) = self
            .tabs
            .iter()
            .find(|tab| tab.id() == editor)
            .and_then(|tab| tab.lsp.as_ref())
            .map(|lsp| lsp.client.encoding())
        else {
            return;
        };
        self.dismiss_problem_panels(cx);
        let cursor = self.editor().cursor(cx);
        let (files, _) = self.apply_workspace_edit(&edit, encoding, cursor, window, cx);
        if files > 1 {
            self.show_note(
                format!("Changed {files} files. Save to keep them."),
                window,
                cx,
            );
        }
    }

    /// Fix the problems on the selected lines, or the cursor's, with the
    /// agent.
    pub(super) fn fix_problems(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.home {
            return;
        }
        if self.previewing() {
            self.show_note(
                "Accept or reject the change under review first.".into(),
                window,
                cx,
            );
            return;
        }
        self.dismiss_problem_panels(cx);
        self.start_conversation(
            Ask {
                name: Some("Fix".into()),
                diagnostics: true,
                ..Ask::new(AGENT_FIX)
            },
            window,
            cx,
        );
    }

    /// Fix the problem starting at `at`, from its panel.
    fn fix_problem_at(&mut self, at: usize, window: &mut Window, cx: &mut Context<Self>) {
        if self.home {
            return;
        }
        let editor = self.editor().clone();
        editor.select(at..at, cx);
        editor.focus(window, cx);
        self.fix_problems(window, cx);
    }

    /// Put away the problem and hover panels, once one of their fixes is
    /// taken.
    fn dismiss_problem_panels(&mut self, cx: &mut Context<Self>) {
        if self.home {
            return;
        }
        let tab = self.tab();
        let (problems, hover) = (tab.problems.hover.clone(), tab.hover.clone());
        problems.update(cx, |problems, cx| problems.dismiss(cx));
        hover.update(cx, |hover, cx| {
            hover.close(cx);
        });
    }

    /// Whether GPUI Kit's code action list is open, which takes Esc.
    pub(super) fn quick_fix_open(&self, cx: &App) -> bool {
        !self.home && self.editor().state().read(cx).code_action_menu_state().open
    }
}

struct QuickFixes {
    workspace: WeakEntity<Workspace>,
    /// The editor this answers for. Only its id: the editor is mid-update
    /// when a fix is taken.
    editor: EntityId,
}

impl CodeActionProvider for QuickFixes {
    fn id(&self) -> SharedString {
        "jig".into()
    }

    fn code_actions(
        &self,
        state: Entity<EditorState>,
        range: Range<usize>,
        window: &mut Window,
        cx: &mut App,
    ) -> Task<Result<Vec<CodeAction>>> {
        let Some(workspace) = self.workspace.upgrade() else {
            return Task::ready(Ok(Vec::new()));
        };
        let state = state.read(cx);
        let text = state.text().to_string();
        let lines = line_span(&text, range);
        let problems = state.diagnostics().is_some_and(|set| {
            set.iter()
                .any(|entry| entry.range.start <= lines.end && entry.range.end >= lines.start)
        });
        let ask = workspace.read(cx).server_fixes(self.editor, lines, cx);
        let workspace = workspace.downgrade();
        window.spawn(cx, async move |cx| {
            let mut actions = match ask {
                Some(ask) => ask(cx).await,
                None => Vec::new(),
            };
            if problems {
                actions.push(jig_action("Fix with Agent", FIX_WITH_AGENT));
            } else if actions.is_empty() {
                workspace
                    .update_in(cx, |this, window, cx| {
                        this.show_note("No problems on this line.".into(), window, cx)
                    })
                    .ok();
            }
            Ok(actions)
        })
    }

    fn perform_code_action(
        &self,
        _state: Entity<EditorState>,
        action: CodeAction,
        _push_to_history: bool,
        window: &mut Window,
        cx: &mut App,
    ) -> Task<Result<()>> {
        let workspace = self.workspace.clone();
        let editor = self.editor;
        // The editor is mid-update; act once it's done.
        window.defer(cx, move |window, cx| {
            workspace
                .update(cx, |this, cx| {
                    match action.command.as_ref().map(|c| c.command.as_str()) {
                        Some(FIX_WITH_AGENT) => this.fix_problems(window, cx),
                        _ => this.apply_fix(editor, &action, window, cx),
                    }
                })
                .ok();
        });
        Task::ready(Ok(()))
    }
}

/// One of Jig's own fixes, told apart from the server's by its command.
fn jig_action(title: &str, command: &str) -> CodeAction {
    CodeAction {
        title: title.into(),
        kind: Some(CodeActionKind::QUICKFIX),
        command: Some(Command {
            title: title.into(),
            command: command.into(),
            arguments: None,
        }),
        ..Default::default()
    }
}

/// The fixes in a `textDocument/codeAction` answer that carry their edits,
/// the server's preferred ones first. Bare commands, and fixes the server
/// says can't be made now, are left out.
fn quick_fixes(answer: &Value) -> Vec<CodeAction> {
    let mut fixes: Vec<CodeAction> = answer
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|item| serde_json::from_value::<CodeActionOrCommand>(item.clone()).ok())
        .filter_map(|item| match item {
            CodeActionOrCommand::CodeAction(action) => Some(action),
            CodeActionOrCommand::Command(_) => None,
        })
        .filter(|action| action.edit.is_some() && action.disabled.is_none())
        .collect();
    fixes.sort_by_key(|action| !action.is_preferred.unwrap_or(false));
    fixes
}

/// The whole lines `range` touches in `text`, from the first's start to the
/// last's end, without its newline.
fn line_span(text: &str, range: Range<usize>) -> Range<usize> {
    let start = range.start.min(text.len());
    let end = range.end.clamp(start, text.len());
    // A selection of whole lines ends after the last one's newline.
    let last = if end > start && text[..end].ends_with('\n') {
        end - 1
    } else {
        end
    };
    let first = text[..start].rfind('\n').map_or(0, |ix| ix + 1);
    let stop = text[last..].find('\n').map_or(text.len(), |ix| last + ix);
    first..stop
}

/// The line `offset` is on, counted from 0 as servers count them.
fn line_of(text: &str, offset: usize) -> u32 {
    text[..offset.min(text.len())].matches('\n').count() as u32
}

/// What a problem's panel offers under its message: the server's fixes,
/// asked for once per problem, then Fix with Agent.
pub(super) struct PanelFixes {
    workspace: WeakEntity<Workspace>,
    editor: EntityId,
    /// The problem offered for: where it starts and what it says.
    problem: Option<(usize, SharedString)>,
    actions: Vec<CodeAction>,
    task: Task<()>,
}

impl PanelFixes {
    pub(super) fn new(workspace: WeakEntity<Workspace>, editor: EntityId) -> Self {
        Self {
            workspace,
            editor,
            problem: None,
            actions: Vec::new(),
            task: Task::ready(()),
        }
    }

    /// Offer fixes for `entry`, asking the server unless it was just asked.
    pub(super) fn offer_for(&mut self, entry: &DiagnosticEntry, cx: &mut Context<Self>) {
        let problem = (entry.range.start, entry.message.clone());
        if self.problem.as_ref() == Some(&problem) {
            return;
        }
        self.problem = Some(problem);
        self.actions.clear();
        let ask = self.workspace.upgrade().and_then(|workspace| {
            workspace
                .read(cx)
                .server_fixes(self.editor, entry.range.clone(), cx)
        });
        self.task = match ask {
            Some(ask) => cx.spawn(async move |this, cx| {
                let actions = ask(cx).await;
                this.update(cx, |this, cx| {
                    this.actions = actions;
                    cx.notify();
                })
                .ok();
            }),
            None => Task::ready(()),
        };
    }

    #[cfg(test)]
    pub(super) fn titles(&self) -> Vec<String> {
        self.actions.iter().map(|a| a.title.clone()).collect()
    }

    fn agent_button(&self, at: usize, cx: &mut Context<Self>) -> Stateful<Div> {
        let accent = surface::agent_accent(cx);
        let workspace = self.workspace.clone();
        h_flex()
            .id("fix-with-agent")
            .debug_selector(|| "fix-with-agent".into())
            .gap_1()
            .px_1p5()
            .py(px(2.))
            .rounded(px(5.))
            .bg(accent.opacity(0.12))
            .hover(|this| this.bg(accent.opacity(0.22)))
            .text_color(accent)
            .text_size(px(11.5))
            .font_weight(FontWeight::MEDIUM)
            .child(surface::agent_icon().size(px(11.)).text_color(accent))
            .child("Fix with Agent")
            .on_click(move |_, window, cx| {
                cx.stop_propagation();
                workspace
                    .update(cx, |this, cx| this.fix_problem_at(at, window, cx))
                    .ok();
            })
    }
}

impl Render for PanelFixes {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some((at, _)) = self.problem.clone() else {
            return Empty.into_any_element();
        };
        let theme = cx.theme();
        let muted = theme.muted_foreground;
        let hover = theme.foreground.opacity(0.06);
        let fixes = self
            .actions
            .iter()
            .take(PANEL_FIXES)
            .enumerate()
            .map(|(ix, action)| {
                let workspace = self.workspace.clone();
                let editor = self.editor;
                let action = action.clone();
                h_flex()
                    .id(("server-fix", ix))
                    .gap_1p5()
                    .px_1()
                    .py(px(2.))
                    .mx_neg_1()
                    .rounded(px(5.))
                    .hover(move |this| this.bg(hover))
                    .child(
                        Icon::default()
                            .data(FIX_ICON)
                            .size(px(12.))
                            .flex_none()
                            .text_color(muted),
                    )
                    .child(div().min_w_0().truncate().child(action.title.clone()))
                    .on_click(move |_, window, cx| {
                        cx.stop_propagation();
                        workspace
                            .update(cx, |this, cx| this.apply_fix(editor, &action, window, cx))
                            .ok();
                    })
            });
        v_flex()
            .gap_1()
            .pt_1p5()
            .border_t_1()
            .border_color(theme.border)
            .children(fixes)
            .child(h_flex().child(self.agent_button(at, cx)))
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{line_span, quick_fixes};

    #[test]
    fn keeps_fixes_with_edits_preferred_first() {
        let edit = json!({"changes": {}});
        let answer = json!([
            {"title": "Run me", "command": "x.run"},
            {"title": "Later", "kind": "quickfix", "edit": edit},
            {"title": "Best", "kind": "quickfix", "edit": edit, "isPreferred": true},
            {"title": "Can't now", "kind": "quickfix", "edit": edit, "disabled": {"reason": "no"}},
            {"title": "No edit", "kind": "quickfix", "command": {"title": "t", "command": "c"}},
        ]);
        let titles: Vec<String> = quick_fixes(&answer).into_iter().map(|a| a.title).collect();
        assert_eq!(titles, ["Best", "Later"]);
        assert!(quick_fixes(&serde_json::Value::Null).is_empty());
    }

    #[test]
    fn spans_whole_lines() {
        let text = "a\nbcd\nef\n";
        assert_eq!(line_span(text, 3..3), 2..5);
        assert_eq!(line_span(text, 0..4), 0..5);
        // Whole lines selected with their newline don't reach the next.
        assert_eq!(line_span(text, 2..6), 2..5);
        assert_eq!(line_span(text, 9..9), 9..9);
    }
}
