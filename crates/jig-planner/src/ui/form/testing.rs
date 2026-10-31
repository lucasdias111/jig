//! What the app's UI tests drive the form with.

use gpui_kit::*;

use super::TaskForm;
use crate::domain::mention::Mentioned;

impl TaskForm {
    pub fn mention_items(&self) -> Option<Vec<String>> {
        self.mention
            .as_ref()
            .map(|mention| mention.items.iter().map(Mentioned::link).collect())
    }

    pub fn index_ready(&self) -> bool {
        self.index.is_some()
    }

    pub fn focus_description(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.description
            .update(cx, |input, cx| input.focus(window, cx));
    }

    pub fn description(&self, cx: &App) -> String {
        self.description.read(cx).value().to_string()
    }

    pub fn fill(
        &mut self,
        title: &str,
        description: &str,
        todos: &[&str],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.title.update(cx, |input, cx| {
            input.set_value(title.to_string(), window, cx)
        });
        self.description.update(cx, |input, cx| {
            input.set_value(description.to_string(), window, cx)
        });
        for todo in todos {
            self.new_todo.update(cx, |input, cx| {
                input.set_value(todo.to_string(), window, cx)
            });
            self.add_todo(window, cx);
        }
    }
}
