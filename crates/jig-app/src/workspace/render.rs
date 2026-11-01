//! Drawing the window: the title bar, the sidebar, the code or the mode showing, and the floating panels over them.

use super::*;

impl Workspace {
    /// The current tab's code, or the Planner or the Map in its place.
    fn render_code(&self, cx: &Context<Self>) -> AnyElement {
        if self.mode == Mode::Agent {
            return self.render_agent_mode(cx);
        }
        if self.mode == Mode::Planner {
            return div()
                .flex_1()
                .min_h_0()
                .child(self.render_planner_mode(cx))
                .into_any_element();
        }
        if let Some(view) = self.map_showing() {
            return div()
                .flex_1()
                .min_h_0()
                .child(view.clone())
                .into_any_element();
        }
        div()
            .flex_1()
            .min_h_0()
            .pl_2()
            .pr_3()
            .pt_1()
            .pb_2()
            // Line shortcuts bind here, not in every other input.
            .key_context("CodeEditor")
            .relative()
            .on_pinch(cx.listener(Self::pinch_code))
            .child(
                Editor::new(self.editor().state())
                    .bordered(false)
                    // The column behind it is the surface.
                    .bg(transparent_black())
                    // Locked while the agent's edit awaits review. The
                    // element re-applies this every frame.
                    .readonly(self.previewing())
                    .context_menu(self.code_menu(cx))
                    .size_full(),
            )
            .children(self.pinch_ruler(cx))
            .into_any_element()
    }
}

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        self.step_sidebar_motion(window);
        let sidebar = self.render_sidebar(cx);
        let title = self.render_title(cx);
        let tab_bar = self.render_tab_bar(cx);
        let root = v_flex();
        let root = self.agent_dock_drag_handlers(root, cx);
        self.run_panel_drag_handlers(self.sidebar_drag_handlers(root, cx), cx)
            .key_context(CONTEXT)
            .relative()
            .size_full()
            // No background here: on macOS the window is translucent and
            // the sidebar lets it show; the editor side paints its own.
            .on_action(cx.listener(Self::quit))
            .on_action(cx.listener(Self::restart_to_update))
            .on_action(cx.listener(Self::open))
            .on_action(cx.listener(Self::save))
            .on_action(cx.listener(Self::save_as))
            .on_action(cx.listener(Self::close_window))
            .on_action(cx.listener(Self::open_command))
            .on_action(cx.listener(Self::send_selection_to_agent))
            .on_action(cx.listener(Self::open_agent_conversations))
            .on_action(cx.listener(Self::edit_agents))
            .on_action(cx.listener(Self::edit_agents_file))
            .on_action(cx.listener(Self::edit_model_config))
            .on_action(cx.listener(Self::toggle_sidebar))
            .on_action(cx.listener(Self::focus_file_tree))
            .on_action(cx.listener(Self::go_to_file))
            .on_action(cx.listener(Self::find))
            .on_action(cx.listener(Self::find_and_replace))
            .on_action(cx.listener(Self::find_in_files))
            .on_action(cx.listener(Self::new_file))
            .on_action(cx.listener(Self::close_tab))
            .on_action(cx.listener(Self::next_tab))
            .on_action(cx.listener(Self::previous_tab))
            .on_action(cx.listener(Self::activate_tab))
            .on_action(cx.listener(Self::run_selected))
            .on_action(cx.listener(Self::choose_run_configuration))
            .on_action(cx.listener(Self::stop_run))
            .on_action(cx.listener(Self::toggle_run_panel))
            .on_action(cx.listener(Self::edit_run_configurations))
            .on_action(cx.listener(Self::edit_debuggers))
            .on_action(cx.listener(Self::debug_selected))
            .on_action(cx.listener(Self::toggle_breakpoint))
            .on_action(cx.listener(Self::resume))
            .on_action(cx.listener(Self::step_over))
            .on_action(cx.listener(Self::step_into))
            .on_action(cx.listener(Self::step_out))
            .on_action(cx.listener(Self::toggle_git_panel))
            .on_action(cx.listener(Self::switch_branch))
            .on_action(cx.listener(Self::new_branch))
            .on_action(cx.listener(Self::rename_symbol))
            .on_action(cx.listener(Self::find_references))
            .on_action(cx.listener(Self::next_problem))
            .on_action(cx.listener(Self::previous_problem))
            .on_action(cx.listener(Self::show_hover))
            .on_action(cx.listener(Self::show_completions))
            .on_action(cx.listener(Self::toggle_line_comment))
            .on_action(cx.listener(Self::format_document))
            .on_action(cx.listener(Self::edit_formatters))
            .on_action(cx.listener(Self::edit_language_servers))
            .on_action(cx.listener(Self::move_line_up))
            .on_action(cx.listener(Self::move_line_down))
            .on_action(cx.listener(Self::duplicate_line))
            .on_action(cx.listener(Self::delete_line))
            .on_action(cx.listener(Self::select_line))
            .on_action(cx.listener(Self::insert_line_below))
            .on_action(cx.listener(Self::insert_line_above))
            .on_action(cx.listener(Self::go_to_line))
            .on_action(cx.listener(Self::zoom_in))
            .on_action(cx.listener(Self::zoom_out))
            .on_action(cx.listener(Self::reset_zoom))
            .on_action(cx.listener(Self::toggle_task_planner))
            .on_action(cx.listener(Self::new_task))
            .on_action(cx.listener(Self::go_back))
            .on_action(cx.listener(Self::go_forward))
            .on_action(cx.listener(Self::show_map))
            .on_action(cx.listener(Self::show_code))
            .on_action(cx.listener(Self::show_agent))
            .on_action(cx.listener(Self::show_planner))
            .on_action(cx.listener(Self::go_to_map))
            .on_action(cx.listener(Self::index_domains))
            .on_action(cx.listener(Self::explain_level))
            .on_action(cx.listener(Self::update_domains))
            .capture_any_mouse_down(cx.listener(Self::navigation_button))
            .capture_action(cx.listener(Self::on_escape))
            .capture_action(cx.listener(Self::on_accept_enter))
            .capture_action(cx.listener(Self::on_accept_tab))
            .capture_action(cx.listener(Self::on_shift_tab))
            .capture_action(cx.listener(Self::on_accept_indent))
            .capture_action(cx.listener(Self::on_undo))
            .on_drag_move(cx.listener(Self::drag_agent_chat))
            .on_drag_move(cx.listener(Self::resize_agent_chat))
            .child(
                // One bar across the whole window, the sidebar below it:
                // the traffic lights, the title, and the command and run
                // controls. The tabs sit in a row below.
                TitleBar::new()
                    .h(px(TITLE_BAR_HEIGHT))
                    .pl_0()
                    .border_b_0()
                    .bg(transparent_black())
                    .child(
                        h_flex()
                            .size_full()
                            .bg(theme::editor_surface(cx))
                            .border_b_1()
                            .border_color(theme.title_bar_border)
                            .child(
                                h_flex()
                                    .flex_1()
                                    .flex_basis(px(0.))
                                    .min_w_0()
                                    .child(div().flex_none().w(px(TRAFFIC_LIGHTS_WIDTH)))
                                    .child(title),
                            )
                            .children(self.render_mode_switch(cx))
                            .child(
                                h_flex()
                                    .flex_1()
                                    .flex_basis(px(0.))
                                    .min_w_0()
                                    .justify_end()
                                    .children(self.render_command_button(cx))
                                    .when(self.mode != Mode::Agent, |this| {
                                        this.children(self.render_run_controls(cx))
                                    })
                                    .child(div().flex_none().w(px(8.))),
                            ),
                    ),
            )
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .items_stretch()
                    .when(self.mode != Mode::Agent, |this| {
                        this.children(self.render_activity_bar(cx))
                            .children(sidebar)
                    })
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .bg(theme::editor_surface(cx))
                            .children(tab_bar)
                            .when(self.start_showing(), |this| {
                                this.child(div().flex_1().min_h_0().child(self.render_start(cx)))
                            })
                            .when(!self.start_showing(), |this| {
                                this.child(
                                    // The find panel floats over the code,
                                    // outside its key context.
                                    div()
                                        .relative()
                                        .flex_1()
                                        .min_h_0()
                                        .flex()
                                        .flex_col()
                                        .child(self.render_code(cx))
                                        .children(self.render_find_panel(cx)),
                                )
                            })
                            .when(self.mode != Mode::Agent, |this| {
                                this.children(self.render_run_panel(cx))
                            })
                            .when(!self.home && self.mode != Mode::Agent, |this| {
                                this.child(self.render_status_bar(cx))
                            }),
                    )
                    .children(self.render_pinned_agent_chat(cx)),
            )
            .when(self.mode != Mode::Agent, |this| {
                this.children(self.render_sidebar_handle(cx))
            })
            .children(self.render_floating_settings(cx))
            .when_some(self.palette.as_ref(), |this, palette| {
                this.child(deferred(
                    anchored()
                        .position(palette.anchor)
                        .snap_to_window_with_margin(px(8.))
                        .child(palette.view.clone()),
                ))
            })
            .children(self.render_task_form(cx))
            .children(self.render_quick_open(cx))
            .children(self.render_find_in_files(cx))
            .children(self.render_rename(cx))
            .children(self.render_go_to_line(cx))
            .children(self.render_run_picker(cx))
            .children(self.render_branch_picker(cx))
            .children(self.render_hunk_popup(cx))
            .when(self.mode == Mode::Code && !self.home, |this| {
                this.child(self.tab().problems.hover.clone())
                    .child(self.tab().hover.clone())
            })
            .when(self.mode == Mode::Code, |this| {
                this.child(self.selection_button.clone())
            })
            // The agent conversation, unless it's docked, and a note.
            .when_some(
                self.chat
                    .as_ref()
                    .filter(|_| !self.floating_agent_chat_pinned())
                    .zip(self.render_agent_chat(cx)),
                |this, (chat, conversation)| {
                    this.child(deferred(
                        anchored()
                            .position(chat.anchor)
                            .snap_to_window_with_margin(px(8.))
                            .child(conversation),
                    ))
                },
            )
            .when_some(self.note.as_ref(), |this, note| {
                this.child(deferred(
                    anchored()
                        .position(note.anchor)
                        .snap_to_window_with_margin(px(8.))
                        .child(note.bubble.clone()),
                ))
            })
    }
}
