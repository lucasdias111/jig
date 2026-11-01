//! The language server's calls and the debugger's stack and logpoints on a Plan.

use super::*;

impl MapView {
    /// Ask the language server about the Plan's function, once.
    pub(super) fn ask_server(&self, cx: &mut Context<Self>) {
        if let Level::Plan(Focus::Function(symbol)) = &self.level
            && let Some(project) = &self.project
            && let Some(id) = project.find(symbol)
            && !project.checked(id)
        {
            cx.emit(MapEvent::AskServer(id));
        }
    }

    /// What the language server says function `id` calls and is called
    /// by: drawn with certainty.
    pub fn server_calls(
        &mut self,
        id: FnId,
        callees: Vec<Target>,
        callers: Vec<FnId>,
        cx: &mut Context<Self>,
    ) {
        let Some(project) = self.project.as_mut() else {
            return;
        };
        Arc::make_mut(project).merge_server(id, callees, callers);
        self.rebuild(true);
        cx.notify();
    }

    /// How far boxes new to the Plan have faded in, from 0 to 1.
    pub(super) fn arriving(&self, key: &str, now: Instant) -> f32 {
        match &self.glide {
            Some((was, start)) if !was.contains_key(key) => {
                (now.duration_since(*start).as_secs_f32() / GLIDE).min(1.)
            }
            _ => 1.,
        }
    }

    /// The debugger paused with this call stack (empty: running, or not
    /// debugging): its functions light up as a path. Pausing in a
    /// function not on the Plan shows its Plan.
    pub fn set_stack(&mut self, stack: Stack, cx: &mut Context<Self>) {
        if stack == self.stack {
            return;
        }
        self.stack = stack;
        self.on_stack = self.stack_functions();
        if let (Some(&paused), Some(project)) = (self.on_stack.first(), self.project.clone())
            && matches!(self.level, Level::Plan(_))
            && self
                .plan
                .index_of(&function_key(&project, paused))
                .is_none()
        {
            self.show(Level::Plan(Focus::Function(project.symbol(paused))), cx);
        }
        cx.notify();
    }

    /// The lines (0-based) the program should report reaching as it
    /// runs, by file: the first statement of each function on the Plan.
    pub fn live_lines(&self) -> Vec<(PathBuf, u32)> {
        let (Some(project), true, Level::Plan(_)) = (&self.project, self.live, &self.level) else {
            return Vec::new();
        };
        self.plan
            .nodes
            .iter()
            .filter_map(Node::function)
            .filter_map(|id| {
                let function = &project.functions[id];
                let line = function.outline.body_line?;
                Some((self.root.join(&project.files[function.file].path), line))
            })
            .collect()
    }

    /// The program reached line `line` (0-based) of `path`: the function
    /// starting there glows.
    pub fn fire_line(&mut self, path: &Path, line: u32, cx: &mut Context<Self>) {
        let Some(project) = self.project.clone() else {
            return;
        };
        let Some(id) = debug::entered(&project, &debug::relative(&self.root, path), line) else {
            return;
        };
        let key = function_key(&project, id);
        if self.plan.index_of(&key).is_none() {
            return;
        }
        let plan = &self.plan;
        self.firing.fire(&key, Instant::now(), |from, to| {
            match (plan.index_of(from), plan.index_of(to)) {
                (Some(from), Some(to)) => plan.wires.iter().any(|w| w.from == from && w.to == to),
                _ => false,
            }
        });
        cx.notify();
    }
}
