//! The Tasks view: making, checking off and editing tasks, tasks agents
//! add, and links from a task to the code, with Go Back and Go Forward.

use super::*;
use crate::planner::TaskPlanner;

const GO_BACK: &str = if cfg!(target_os = "macos") {
    "ctrl--"
} else {
    "alt-left"
};

const GO_FORWARD: &str = if cfg!(target_os = "macos") {
    "ctrl-shift--"
} else {
    "alt-right"
};

/// A project with two files, opened as a folder with `b.rs` showing.
fn project(cx: &mut TestAppContext) -> (tempfile::TempDir, AnyWindowHandle, Entity<Workspace>) {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("src");
    std::fs::create_dir(&src).unwrap();
    std::fs::write(
        src.join("a.rs"),
        "use crate::b;\n\nfn run() {\n    b::go();\n}\n",
    )
    .unwrap();
    std::fs::write(src.join("b.rs"), "pub fn go() {}\n\nfn other() {}\n").unwrap();
    let (window, workspace) = open(cx, dir.path());
    open_file(cx, window, &workspace, &src.join("b.rs"));
    (dir, window, workspace)
}

fn planner(cx: &mut TestAppContext, workspace: &Entity<Workspace>) -> Entity<TaskPlanner> {
    cx.update(|cx| {
        workspace
            .read(cx)
            .planner_view()
            .cloned()
            .expect("a planner")
    })
}

fn task_ids(cx: &mut TestAppContext, planner: &Entity<TaskPlanner>) -> Vec<String> {
    cx.update(|cx| {
        planner
            .read(cx)
            .tasks()
            .iter()
            .map(|task| task.id.clone())
            .collect()
    })
}

fn current_file(cx: &mut TestAppContext, workspace: &Entity<Workspace>) -> String {
    cx.update(|cx| {
        workspace
            .read(cx)
            .document()
            .path
            .as_ref()
            .and_then(|path| path.file_name())
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default()
    })
}

#[gpui_kit::test]
fn the_tasks_view_makes_checks_off_and_edits_tasks(cx: &mut TestAppContext) {
    let (dir, window, workspace) = project(cx);
    step(cx, window, |window, cx| {
        window.render_frame(cx);
        window.press("ctrl-shift-t", cx);
    });
    assert!(cx.update(|cx| workspace.read(cx).planner_shown()));
    let planner = planner(cx, &workspace);
    assert!(task_ids(cx, &planner).is_empty());

    // A new task, from the form.
    let view = planner.clone();
    step(cx, window, move |window, cx| {
        view.update(cx, |planner, cx| {
            planner.new_task(window, cx);
            planner.fill_form(
                "Add a planner",
                "Starts in [`run`](src/a.rs#L3).",
                &["Store tasks", "Show them"],
                window,
                cx,
            );
        });
        window.render_frame(cx);
    });
    let ws = workspace.clone();
    let focused = cx
        .update_window(window, move |_, window, cx| {
            ws.read(cx).planner_focused(window, cx)
        })
        .unwrap();
    assert!(focused, "the form has the keyboard");
    // ⌘↩ in the form saves; the workspace leaves the key to it.
    step(cx, window, |window, cx| window.press("secondary-enter", cx));
    let file = dir.path().join(".jig/tasks/add-a-planner.md");
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        "# Add a planner\n\nStarts in [`run`](src/a.rs#L3).\n\n## Todo\n\n- [ ] Store tasks\n- [ ] Show them\n"
    );
    assert_eq!(task_ids(cx, &planner), ["add-a-planner"]);
    assert!(cx.update(|cx| {
        let planner = planner.read(cx);
        !planner.form_open() && planner.is_expanded("add-a-planner")
    }));

    // Checking a todo off writes it.
    cx.update(|cx| {
        planner.update(cx, |planner, cx| {
            planner.toggle_todo("add-a-planner", 0, cx)
        })
    });
    assert!(
        std::fs::read_to_string(&file)
            .unwrap()
            .contains("- [x] Store tasks")
    );

    // A todo added under the open task.
    let view = planner.clone();
    step(cx, window, move |window, cx| {
        view.update(cx, |planner, cx| {
            planner.start_adding_todo("add-a-planner", window, cx)
        });
    });
    step(cx, window, |window, cx| {
        window.render_frame(cx);
        window.input("Link the code", cx);
    });
    step(cx, window, |window, cx| window.press("enter", cx));
    assert!(
        std::fs::read_to_string(&file)
            .unwrap()
            .ends_with("- [ ] Link the code\n")
    );

    // Edited: the same file, renamed in its title only; Esc leaves a form.
    let view = planner.clone();
    step(cx, window, move |window, cx| {
        view.update(cx, |planner, cx| {
            planner.edit_task("add-a-planner", window, cx);
            planner.fill_form("A task planner", "Starts in a.rs.", &[], window, cx);
            planner.save_form(window, cx);
            planner.edit_task("add-a-planner", window, cx);
        });
        window.render_frame(cx);
    });
    step(cx, window, |window, cx| window.press("escape", cx));
    assert!(!cx.update(|cx| planner.read(cx).form_open()), "Esc cancels");
    let task = jig_ai::tasks::load(dir.path(), "add-a-planner").unwrap();
    assert_eq!(task.title, "A task planner");
    assert_eq!(task.description, "Starts in a.rs.");
    assert_eq!(task.todos.len(), 3, "the todos stay");
    assert!(task.todos[0].done);
}

#[gpui_kit::test]
fn tasks_from_agents_show_up_and_are_pointed_out(cx: &mut TestAppContext) {
    let (dir, window, workspace) = project(cx);
    let planner = planner(cx, &workspace);
    // What the agent's tool does.
    jig_ai::tasks::create(dir.path(), "From the agent", "", vec![]).unwrap();
    step(cx, window, |window, cx| {
        workspace.update(cx, |this, cx| this.tasks_changed(window, cx))
    });
    assert_eq!(task_ids(cx, &planner), ["from-the-agent"]);
    assert!(
        cx.update(|cx| workspace.read(cx).planner.unseen),
        "hidden, so pointed out"
    );
    assert!(cx.update(|cx| planner.read(cx).is_expanded("from-the-agent")));

    step(cx, window, |window, cx| {
        window.render_frame(cx);
        window.press("ctrl-shift-t", cx);
    });
    assert!(!cx.update(|cx| workspace.read(cx).planner.unseen), "seen");
}

#[gpui_kit::test]
fn task_links_open_the_code_and_go_back_and_forth(cx: &mut TestAppContext) {
    let (_dir, window, workspace) = project(cx);
    let planner = planner(cx, &workspace);
    let other = "pub fn go() {}\n\n".len();
    cx.update(|cx| {
        let editor = workspace.read(cx).editor().clone();
        editor.select(other..other, cx);
    });

    // The link's line is out of date; the name is found anyway.
    let markdown = "Starts in [`run`](src/a.rs#L1).";
    let view = planner.clone();
    step(cx, window, move |_, cx| {
        view.update(cx, |planner, cx| {
            planner.click_link("src/a.rs#L1", markdown, cx)
        })
    });
    assert_eq!(current_file(cx, &workspace), "a.rs");
    let run = "use crate::b;\n\nfn ".len();
    assert_eq!(
        cx.update(|cx| workspace.read(cx).editor().selection(cx)),
        run..run + 3
    );

    step(cx, window, |window, cx| {
        window.render_frame(cx);
        window.press(GO_BACK, cx);
    });
    assert_eq!(current_file(cx, &workspace), "b.rs");
    assert_eq!(
        cx.update(|cx| workspace.read(cx).editor().cursor(cx)),
        other
    );
    assert_eq!(cx.update(|cx| workspace.read(cx).history_lengths()), (0, 1));

    step(cx, window, |window, cx| {
        window.render_frame(cx);
        window.press(GO_FORWARD, cx);
    });
    assert_eq!(current_file(cx, &workspace), "a.rs");
    // Back where the cursor was: after the name the link selected.
    assert_eq!(
        cx.update(|cx| workspace.read(cx).editor().cursor(cx)),
        run + 3
    );

    // Go to Line is a jump too.
    step(cx, window, |window, cx| {
        window.dispatch_action(Box::new(crate::workspace::GoToLine), cx)
    });
    step(cx, window, |window, cx| window.input("4", cx));
    step(cx, window, |window, cx| window.press("enter", cx));
    let line_4 = "use crate::b;\n\nfn run() {\n".len();
    assert_eq!(
        cx.update(|cx| workspace.read(cx).editor().cursor(cx)),
        line_4
    );
    step(cx, window, |window, cx| window.press(GO_BACK, cx));
    assert_eq!(
        cx.update(|cx| workspace.read(cx).editor().cursor(cx)),
        run + 3
    );
    assert_eq!(
        text(cx, &workspace),
        "use crate::b;\n\nfn run() {\n    b::go();\n}\n",
        "nothing typed into the code"
    );
}
