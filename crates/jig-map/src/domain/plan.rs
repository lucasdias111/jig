//! The Plan level: a neighbourhood around a focus, never the whole
//! project. A function's is its callers on the left and its callees on
//! the right, one hop out, more opened on demand; a module's (a folder, a
//! file or a type) is its functions framed by type, called ones right of
//! their callers, with the calls in from elsewhere on the left and out to
//! elsewhere on the right. Functions sit in frames for their type or
//! file. A file the index can't read (a Svelte component, a page) is
//! one box with what links to it on the left and what it links to on the
//! right, its functions not read.
//!
//! What would drown the picture is folded: past 8 callers or callees on a
//! side the rest are one "+N more" box, a side's tests are one box, calls
//! into a package outside the project are one box per package, and
//! functions called from all over (hubs) are badges on their callers.

use std::collections::{HashMap, HashSet};

use super::files::Sources;
use super::index::{FnId, Func, Project, Symbol, Target};
use super::outline::simple_type;
mod module;
mod nodes;
mod sides;
mod spread;
pub use module::*;
pub use nodes::*;
use sides::*;
use spread::*;

/// Callers or callees shown on a side before the rest fold away.
pub const MAX_SIDE: usize = 8;
/// A module's own functions shown before the rest fold away.
pub const MAX_MODULE: usize = 80;

/// What a Plan is about.
#[derive(Clone, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum Focus {
    /// A function, coming from the code or another function.
    Function(Symbol),
    /// A module, coming from Site: a folder (`dir/`), a file (`path`) or a
    /// type in one (`path#Type`).
    Module(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Item {
    Function(FnId),
    /// A function a Plan was about that's no longer found.
    Gone(Symbol),
    /// Calls into a package outside the project.
    External(String),
    /// A file, as a whole: one the index can't read, or one linked to it.
    File {
        path: String,
        read: bool,
    },
    /// Functions folded away: on a side, or the rest of a module.
    More {
        ids: Vec<FnId>,
        key: String,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Node {
    /// What keeps it across rebuilds, and in saved places.
    pub key: String,
    pub item: Item,
    pub frame: Option<usize>,
    /// Left to right: callers before callees.
    pub column: i32,
    pub title: String,
    /// Below the title: a compact signature, a package's call count.
    pub detail: String,
    /// Hubs it calls, folded: `log ×3`.
    pub badges: Vec<String>,
    /// A line under it all, in words: why a file's functions aren't shown.
    pub note: String,
    /// Callers and callees not drawn, for its expand buttons.
    pub hidden_callers: usize,
    pub hidden_callees: usize,
    /// For an interface's method: its implementations, not drawn yet.
    pub implementations: usize,
    /// It calls a function it was handed, whatever that is.
    pub unknown_target: bool,
}

impl Node {
    fn new(key: String, item: Item, title: String, column: i32) -> Self {
        Node {
            key,
            item,
            frame: None,
            column,
            title,
            detail: String::new(),
            badges: Vec::new(),
            note: String::new(),
            hidden_callers: 0,
            hidden_callees: 0,
            implementations: 0,
            unknown_target: false,
        }
    }

    pub fn function(&self) -> Option<FnId> {
        match self.item {
            Item::Function(id) => Some(id),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WireKind {
    /// A calls B.
    Call,
    /// A calls something named like B, one of a few.
    Ambiguous,
    /// A hands B on, to be called later.
    Callback,
}

impl WireKind {
    /// Drawn dashed: a guess, or a function handed on.
    pub fn dashed(&self) -> bool {
        matches!(self, WireKind::Ambiguous | WireKind::Callback)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Wire {
    pub from: usize,
    pub to: usize,
    pub kind: WireKind,
}

/// A type or file the nodes in it sit inside.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frame {
    pub key: String,
    pub title: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Plan {
    pub nodes: Vec<Node>,
    pub wires: Vec<Wire>,
    pub frames: Vec<Frame>,
    /// The function a function's Plan is about.
    pub focus: Option<usize>,
}

impl Plan {
    pub fn index_of(&self, key: &str) -> Option<usize> {
        self.nodes.iter().position(|node| node.key == key)
    }

    fn frame(&mut self, key: String, title: String) -> usize {
        match self.frames.iter().position(|frame| frame.key == key) {
            Some(ix) => ix,
            None => {
                self.frames.push(Frame { key, title });
                self.frames.len() - 1
            }
        }
    }

    fn wire(&mut self, from: usize, to: usize, kind: WireKind) {
        let wire = Wire { from, to, kind };
        if from != to && !self.wires.contains(&wire) {
            self.wires.push(wire);
        }
    }
}

/// What's opened in a Plan beyond its first hop.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Opened {
    /// Functions whose callers, or callees, are shown too.
    pub callers: HashSet<String>,
    pub callees: HashSet<String>,
    /// Interface methods whose implementations are shown.
    pub implementations: HashSet<String>,
    /// "+N more" groups opened.
    pub more: HashSet<String>,
}

/// The Plan of `focus`, with what's opened past its first hop.
pub fn plan(project: &Project, sources: &dyn Sources, focus: &Focus, opened: &Opened) -> Plan {
    match focus {
        Focus::Function(symbol) => match project.find(symbol) {
            Some(id) => neighbourhood(project, id, opened),
            None => gone(symbol),
        },
        Focus::Module(place) if project.others.iter().any(|other| slashed(other) == *place) => {
            unread_file(project, sources, place)
        }
        Focus::Module(place) => module(project, place, opened),
    }
}

/// Function `focus` with its callers and callees, and what's opened past
/// them.
pub fn neighbourhood(project: &Project, focus: FnId, opened: &Opened) -> Plan {
    let mut plan = Plan::default();
    let at = add_function(&mut plan, project, focus, 0);
    plan.focus = Some(at);
    let key = function_key(project, focus);
    spread(&mut plan, project, &key, &[focus], opened);
    finish(&mut plan, project);
    plan
}

/// A path as the Map's keys write it, with forward slashes.
pub fn slashed(path: &std::path::Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

/// What a Plan about function `symbol` shows once it's no longer found.
pub fn gone(symbol: &Symbol) -> Plan {
    let mut node = Node::new(
        format!("gone:{}#{}", symbol.file, symbol.title()),
        Item::Gone(symbol.clone()),
        symbol.title(),
        0,
    );
    node.detail = "gone: no longer found".into();
    Plan {
        nodes: vec![node],
        ..Plan::default()
    }
}

#[cfg(test)]
mod tests;
