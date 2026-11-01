//! The output panel's Debugger view: the call stack and the variables tree.

use super::*;

/// One row of the variables tree.
pub(super) struct VariableRow {
    pub(super) depth: usize,
    pub(super) variable: Variable,
    pub(super) expanded: bool,
}

impl Workspace {
    /// The Debugger view: the call stack beside the variables.
    pub(in crate::workspace) fn render_debugger(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let debug = self.debug().filter(|d| d.view == View::Debugger)?;
        let theme = cx.theme();
        let message = |text: &'static str| {
            div()
                .p_3()
                .text_size(px(12.))
                .text_color(theme.muted_foreground)
                .child(text)
                .into_any_element()
        };
        if debug.phase != Phase::Paused {
            let text = match debug.phase {
                Phase::Building | Phase::Starting => "Starting…",
                Phase::Running => "Running. The call stack and variables show when it pauses.",
                _ => "Not paused.",
            };
            return Some(message(text));
        }
        let frames = uniform_list(
            "debug-frames",
            debug.frames.len(),
            cx.processor(|this, range: Range<usize>, _, cx| {
                range
                    .map(|ix| this.render_frame(ix, cx))
                    .collect::<Vec<_>>()
            }),
        )
        .track_scroll(&debug.frames_scroll)
        .size_full();
        let rows = variable_rows(debug).len();
        let variables = uniform_list(
            "debug-variables",
            rows,
            cx.processor(|this, range: Range<usize>, _, cx| {
                let Some(debug) = this.debug() else {
                    return Vec::new();
                };
                let rows = variable_rows(debug);
                range
                    .filter_map(|ix| rows.get(ix).map(|row| this.render_variable(ix, row, cx)))
                    .collect::<Vec<_>>()
            }),
        )
        .track_scroll(&debug.variables_scroll)
        .size_full();
        Some(
            h_flex()
                .flex_1()
                .min_h_0()
                .items_stretch()
                .text_size(px(12.))
                .child(
                    div()
                        .w(relative(0.38))
                        .flex_none()
                        .h_full()
                        .py_1()
                        .border_r_1()
                        .border_color(theme.title_bar_border)
                        .child(frames),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .h_full()
                        .py_1()
                        .font_family(theme.mono_font_family.clone())
                        .child(variables),
                )
                .into_any_element(),
        )
    }

    fn render_frame(&self, ix: usize, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let Some(debug) = self.debug() else {
            return div().into_any_element();
        };
        let frame = &debug.frames[ix];
        let selected = ix == debug.selected_frame;
        let location = frame.path.as_ref().map(|path| {
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            format!("{name}:{}", frame.line)
        });
        h_flex()
            .id(("debug-frame", ix))
            .h(px(ROW_HEIGHT))
            .px_3()
            .gap_2()
            .overflow_hidden()
            .when(selected, |this| this.bg(theme.blue.opacity(0.16)))
            .when(!selected, |this| {
                this.hover(|s| s.bg(theme.foreground.opacity(0.05)))
            })
            .text_color(if frame.path.is_some() {
                theme.foreground
            } else {
                theme.muted_foreground
            })
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .child(frame.name.clone()),
            )
            .children(location.map(|location| {
                div()
                    .flex_none()
                    .text_color(theme.muted_foreground)
                    .child(location)
            }))
            .on_click(cx.listener(move |this, _, window, cx| this.select_frame(ix, window, cx)))
            .into_any_element()
    }

    fn render_variable(&self, ix: usize, row: &VariableRow, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let variable = &row.variable;
        let reference = variable.reference;
        let scope = row.depth == 0;
        let chevron = (reference > 0).then(|| {
            Icon::default()
                .data(if row.expanded {
                    CHEVRON_DOWN
                } else {
                    CHEVRON_RIGHT
                })
                .size(px(10.))
                .text_color(theme.muted_foreground)
        });
        h_flex()
            .id(("debug-variable", ix))
            .h(px(ROW_HEIGHT))
            .pl(px(10. + row.depth.saturating_sub(1) as f32 * 14.))
            .pr_3()
            .gap_1()
            .overflow_hidden()
            .whitespace_nowrap()
            .hover(|s| s.bg(theme.foreground.opacity(0.05)))
            .child(div().flex_none().w(px(12.)).children(chevron))
            .child(
                div()
                    .flex_none()
                    .text_color(if scope {
                        theme.muted_foreground
                    } else {
                        theme.foreground
                    })
                    .when(scope, |this| this.font_weight(FontWeight::SEMIBOLD))
                    .child(variable.name.clone()),
            )
            .when(!scope, |this| {
                this.child(
                    div()
                        .flex_none()
                        .text_color(theme.muted_foreground)
                        .child("="),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_color(theme.blue)
                        .child(variable.value.clone()),
                )
                .children(variable.kind.clone().map(|kind| {
                    div()
                        .flex_none()
                        .max_w(px(220.))
                        .truncate()
                        .text_color(theme.muted_foreground)
                        .child(kind)
                }))
            })
            .when(reference > 0, |this| {
                this.on_click(
                    cx.listener(move |this, _, _, cx| this.toggle_variable(reference, cx)),
                )
            })
            .into_any_element()
    }
}

pub(super) fn variable_rows(debug: &DebugSession) -> Vec<VariableRow> {
    fn walk(
        debug: &DebugSession,
        variables: &[Variable],
        depth: usize,
        rows: &mut Vec<VariableRow>,
    ) {
        for variable in variables {
            let expanded = variable.reference > 0 && debug.expanded.contains(&variable.reference);
            rows.push(VariableRow {
                depth,
                variable: variable.clone(),
                expanded,
            });
            if expanded && let Some(children) = debug.children.get(&variable.reference) {
                walk(debug, children, depth + 1, rows);
            }
        }
    }
    let mut rows = Vec::new();
    walk(debug, &debug.scopes, 0, &mut rows);
    rows
}
