//! The debugger on the Map's Plan. Paused, the function it's paused in
//! and the functions calling it, down its call stack, light up as a path.
//! Running, each function glows as it's entered and fades, as in Unreal,
//! and so does the call it came along: the debugger reports reaching a
//! function's first statement without stopping (a logpoint). Stepping
//! happens in the code.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use super::index::{FnId, Project};

/// How long a box or call stays lit after firing.
pub const FADE: Duration = Duration::from_millis(1200);
/// How long a pulse takes to run down a call.
pub const PULSE: Duration = Duration::from_millis(350);

/// The paused program's call stack, innermost first: each frame's file
/// and line (0-based).
pub type Stack = Vec<(PathBuf, u32)>;

/// The functions of the project on `stack`, innermost first, each once:
/// frames outside the project, or in code the index doesn't read, are
/// left out.
pub fn on_stack(project: &Project, stack: &[(PathBuf, u32)]) -> Vec<FnId> {
    let mut found: Vec<FnId> = Vec::new();
    for (path, line) in stack {
        let relative = relative(&project.root, path);
        if let Some(id) = project.function_on_line(&relative, *line)
            && !found.contains(&id)
        {
            found.push(id);
        }
    }
    found
}

/// `path` relative to `root`, as the index names files.
pub fn relative(root: &Path, path: &Path) -> PathBuf {
    let canonical = |path: &Path| path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    path.strip_prefix(root)
        .map(Path::to_path_buf)
        .unwrap_or_else(|_| {
            canonical(path)
                .strip_prefix(canonical(root))
                .map(Path::to_path_buf)
                .unwrap_or_else(|_| path.to_path_buf())
        })
}

/// The function whose body starts on line `line` (0-based) of `file`
/// (relative): where its logpoint is.
pub fn entered(project: &Project, file: &Path, line: u32) -> Option<FnId> {
    let file = project.files.iter().position(|f| f.path == file)?;
    (0..project.functions.len()).find(|&id| {
        let function = &project.functions[id];
        function.file == file && function.outline.body_line == Some(line)
    })
}

/// When each box and call last fired, by key.
#[derive(Clone, Debug, Default)]
pub struct Firing {
    boxes: HashMap<String, Instant>,
    calls: HashMap<(String, String), Instant>,
    /// The box that fired last, which the next one was called from if
    /// there's a call between them.
    last: Option<String>,
}

impl Firing {
    /// Box `key` fired at `now`; `calls(from, to)` says whether one box
    /// calls another on the Plan, so the call it came along lights too.
    pub fn fire(&mut self, key: &str, now: Instant, calls: impl Fn(&str, &str) -> bool) {
        if let Some(from) = self.last.take()
            && from != key
            && calls(&from, key)
        {
            self.calls.insert((from, key.to_string()), now);
        }
        self.boxes.insert(key.to_string(), now);
        self.last = Some(key.to_string());
    }

    /// Another Plan is drawn: nothing on it has fired.
    pub fn clear(&mut self) {
        *self = Self::default();
    }

    /// How brightly box `key` shows, from 1 as it fires to 0.
    pub fn of_box(&self, key: &str, now: Instant) -> f32 {
        strength(self.boxes.get(key), now)
    }

    /// How brightly the call from `from` to `to` shows, and how far down
    /// it the pulse is, while it runs.
    pub fn of_call(&self, from: &str, to: &str, now: Instant) -> (f32, Option<f32>) {
        let Some(&at) = self.calls.get(&(from.to_string(), to.to_string())) else {
            return (0., None);
        };
        let age = now.saturating_duration_since(at);
        let pulse = (age < PULSE).then(|| age.as_secs_f32() / PULSE.as_secs_f32());
        (strength(Some(&at), now), pulse)
    }

    /// Whether anything is still lit, so the view keeps drawing.
    pub fn lit(&self, now: Instant) -> bool {
        self.boxes
            .values()
            .chain(self.calls.values())
            .any(|&at| now.saturating_duration_since(at) < FADE)
    }
}

fn strength(at: Option<&Instant>, now: Instant) -> f32 {
    let Some(&at) = at else {
        return 0.;
    };
    let age = now.saturating_duration_since(at).as_secs_f32();
    (1. - age / FADE.as_secs_f32()).max(0.)
}

#[cfg(test)]
mod tests {
    use super::super::index::tests::{fixture, id};
    use super::*;

    #[test]
    fn a_stack_is_its_functions_innermost_first() {
        let dir = fixture();
        let project = Project::read(dir.path());
        let file = |name: &str| dir.path().join(name);
        // In `find`, called from `create`; a frame outside the project.
        let stack = vec![
            (file("server/src/org/demo/users/UserService.java"), 4),
            (file("server/src/org/demo/bugs/BugService.java"), 11),
            (PathBuf::from("/usr/lib/jvm/Thread.java"), 120),
        ];
        assert_eq!(
            on_stack(&project, &stack),
            [
                id(&project, "UserService.find"),
                id(&project, "BugService.create")
            ]
        );
    }

    #[test]
    fn a_function_is_entered_on_its_first_statement() {
        let dir = fixture();
        let project = Project::read(dir.path());
        let users = Path::new("server/src/org/demo/users/UserService.java");
        assert_eq!(
            entered(&project, users, 4),
            Some(id(&project, "UserService.find"))
        );
        assert_eq!(entered(&project, users, 3), None, "its signature's line");
    }

    #[test]
    fn firing_lights_the_box_and_the_call_it_came_along() {
        let mut firing = Firing::default();
        let now = Instant::now();
        let calls = |from: &str, to: &str| from == "a" && to == "b";
        firing.fire("a", now, calls);
        firing.fire("b", now, calls);
        assert_eq!(firing.of_box("b", now), 1.);
        assert_eq!(firing.of_call("a", "b", now), (1., Some(0.)));
        // Not called from c: no call lights.
        firing.fire("c", now, calls);
        assert_eq!(firing.of_call("b", "c", now).0, 0.);
        let later = now + FADE;
        assert_eq!(firing.of_box("a", later), 0.);
        assert!(!firing.lit(later));
    }
}
