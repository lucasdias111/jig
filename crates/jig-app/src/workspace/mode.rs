//! What the workspace shows: Code, Planner, Map or Agent. The
//! Agent page hides the code chrome. Each mode keeps
//! its place while another shows: Code its tabs and cursor, the Planner
//! its checklist (kept per window), the Map its level and camera, and
//! the agent its conversation and draft.
//! [`Workspace::set_mode`] is the one way between them; anything that
//! puts the cursor in the code (another tab, Go to Line, a problem, a
//! task's link) shows Code again through it.

use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{ActiveTheme as _, Icon, IconName, h_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use jig_editor::EditorHandle as _;

use super::{GoToMap, ShowAgent, ShowCode, ShowPlanner, Workspace};

/// Lucide "code".
const CODE_ICON: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="m16 18 6-6-6-6"/><path d="m8 6-6 6 6 6"/></svg>"#;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum Mode {
    #[default]
    Code,
    Planner,
    Map,
    Agent,
}

impl Mode {
    const ALL: [Mode; 4] = [Mode::Code, Mode::Planner, Mode::Map, Mode::Agent];

    fn label(self) -> &'static str {
        match self {
            Mode::Code => "Code",
            Mode::Planner => "Planner",
            Mode::Map => "Map",
            Mode::Agent => "Agent",
        }
    }

    /// What its segment's tooltip says, with its keys.
    fn tooltip(self) -> &'static str {
        let mac = cfg!(target_os = "macos");
        match (self, mac) {
            (Mode::Agent, true) => "Full-screen agent (⌘4)",
            (Mode::Agent, false) => "Full-screen agent (Ctrl 4)",
            (Mode::Code, true) => "The code (⌘1)",
            (Mode::Code, false) => "The code (Ctrl 1)",
            (Mode::Planner, true) => "The tasks (⌘2)",
            (Mode::Planner, false) => "The tasks (Ctrl 2)",
            (Mode::Map, true) => "How the code at the cursor fits in (⌘3 or ⌘B)",
            (Mode::Map, false) => "How the code at the cursor fits in (Ctrl 3 or Ctrl B)",
        }
    }
}

impl Workspace {
    /// Show `mode` in the editor's place, keeping the place of the one
    /// left, and give it the keyboard: the code, the checklist or the canvas.
    /// The Map flies out from the cursor, as ⌘B does.
    pub(super) fn set_mode(&mut self, mode: Mode, window: &mut Window, cx: &mut Context<Self>) {
        if mode == self.mode {
            return self.focus_main(window, cx);
        }
        match mode {
            Mode::Code => {
                self.switch_mode(Mode::Code, cx);
                self.focus_main(window, cx);
            }
            Mode::Planner => {
                if self.tree.is_none() {
                    return self.show_error("Open a project first, to plan its tasks.", window, cx);
                }
                // Go Back from the Planner returns to where this was.
                self.remember_location(cx);
                self.leave_tab(cx);
                self.switch_mode(Mode::Planner, cx);
                self.show_planner_mode(window, cx);
            }
            Mode::Map => self.map_from_code(window, cx),
            Mode::Agent => {
                if self.agent_project(cx).is_none() {
                    return self.show_error(
                        "Open a project first, to work with the agent.",
                        window,
                        cx,
                    );
                }
                self.remember_location(cx);
                self.leave_tab(cx);
                self.palette = None;
                self.close_find_panel(cx);
                self.switch_mode(Mode::Agent, cx);
                self.show_agent_mode(window, cx);
            }
        }
    }

    /// ⌘4: the full-screen agent.
    pub(super) fn show_agent(
        &mut self,
        _: &ShowAgent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.set_mode(Mode::Agent, window, cx);
    }

    /// ⌘1: the code.
    pub(super) fn show_code(&mut self, _: &ShowCode, window: &mut Window, cx: &mut Context<Self>) {
        self.set_mode(Mode::Code, window, cx);
    }

    /// ⌘2: the Planner.
    pub(super) fn show_planner(
        &mut self,
        _: &ShowPlanner,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.set_mode(Mode::Planner, window, cx);
    }

    /// ⌘3: the Map, flown out of the cursor; pressed while it shows, it
    /// stays.
    pub(super) fn go_to_map(&mut self, _: &GoToMap, window: &mut Window, cx: &mut Context<Self>) {
        self.set_mode(Mode::Map, window, cx);
    }

    /// Give the current tab's code the keyboard, showing it in place of
    /// the Planner or the Map.
    pub(super) fn focus_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.switch_mode(Mode::Code, cx);
        self.editor().focus(window, cx);
    }

    /// Change the mode without moving the keyboard: the mode left keeps
    /// its place. What arrives sets itself up.
    pub(super) fn switch_mode(&mut self, mode: Mode, cx: &mut Context<Self>) {
        if mode == self.mode {
            return;
        }
        let left = std::mem::replace(&mut self.mode, mode);
        if left == Mode::Map {
            if let Some(pane) = &self.map {
                pane.view.read(cx).save_view();
            }
            self.sync_live_points(cx);
        }
        cx.notify();
    }
}

impl Workspace {
    /// Centred in the title bar: Code, Planner, Map and Agent,
    /// the one showing pressed. Planner has a dot when a task arrives while hidden.
    pub(super) fn render_mode_switch(&self, cx: &Context<Self>) -> Option<AnyElement> {
        if self.home && self.tree.is_none() && self.mode == Mode::Code {
            return None;
        }
        let theme = cx.theme();
        let accent = theme.primary;
        let unseen = self.planner.unseen;
        let segment = |mode: Mode| {
            let pressed = self.mode == mode;
            let accent = if mode == Mode::Agent {
                jig_commands::surface::agent_accent(cx)
            } else {
                accent
            };
            let icon = match mode {
                Mode::Code => Icon::default().data(CODE_ICON),
                Mode::Planner => Icon::default().data(crate::planner::TASKS_ICON),
                Mode::Map => Icon::new(IconName::Map),
                Mode::Agent => jig_commands::surface::agent_icon(),
            };
            let selector = format!("mode-{}", mode.label().to_lowercase());
            h_flex()
                .id(selector.clone())
                .debug_selector(move || selector.clone())
                .relative()
                .flex_none()
                .h_full()
                .px_3()
                .justify_center()
                .gap_1p5()
                .rounded(px(5.))
                .text_color(theme.foreground.opacity(0.85))
                .when(pressed, |this| {
                    this.bg(accent.opacity(0.16)).text_color(accent)
                })
                .when(!pressed, |this| {
                    this.hover(|style| style.bg(theme.foreground.opacity(0.08)))
                })
                .child(icon.size(px(13.)).flex_none().text_color(accent))
                .child(div().flex_none().whitespace_nowrap().child(mode.label()))
                .when(mode == Mode::Planner && unseen, |this| {
                    this.child(
                        div()
                            .debug_selector(|| "mode-planner-dot".into())
                            .absolute()
                            .top(px(3.))
                            .right(px(3.))
                            .size(px(6.))
                            .rounded_full()
                            .bg(jig_commands::surface::agent_accent(cx)),
                    )
                })
                .tooltip(move |window, cx| Tooltip::new(mode.tooltip()).build(window, cx))
                // Otherwise the title bar takes the press as a window drag.
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(cx.listener(move |this, _, window, cx| this.set_mode(mode, window, cx)))
        };
        Some(
            h_flex()
                .flex_none()
                .debug_selector(|| "mode-switch".into())
                .h(px(32.))
                .p(px(2.))
                .gap_0p5()
                .rounded(px(7.))
                .border_1()
                .border_color(theme.foreground.opacity(0.1))
                .bg(theme.foreground.opacity(0.04))
                .font_weight(FontWeight::MEDIUM)
                .text_size(px(13.))
                .children(Mode::ALL.map(segment))
                .into_any_element(),
        )
    }
}
