//! Where a Plan's boxes and frames go, in unzoomed points: a column per
//! hop (callers left, callees right), each column's boxes grouped in
//! their frames, columns centred on each other so a neighbourhood fans
//! out from its function.

use super::plan::{Item, Node, Plan};

pub const NODE_WIDTH: f32 = 250.;
pub const HEADER_HEIGHT: f32 = 28.;
pub const ROW_HEIGHT: f32 = 18.;
pub const PADDING: f32 = 6.;
const FRAME_PAD: f32 = 12.;
pub const FRAME_TITLE: f32 = 24.;
const NODE_GAP: f32 = 14.;
const GROUP_GAP: f32 = 30.;
const COLUMN_GAP: f32 = 90.;
/// A column taller than this goes on in another beside it.
const MAX_COLUMN: f32 = 1600.;

/// A box on the canvas, in unzoomed points.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Rect {
    pub fn right(&self) -> f32 {
        self.x + self.width
    }

    pub fn bottom(&self) -> f32 {
        self.y + self.height
    }

    pub fn centre(&self) -> (f32, f32) {
        (self.x + self.width / 2., self.y + self.height / 2.)
    }

    pub fn contains(&self, (x, y): (f32, f32)) -> bool {
        x >= self.x && x <= self.right() && y >= self.y && y <= self.bottom()
    }

    pub fn overlaps(&self, other: &Rect) -> bool {
        self.x < other.right()
            && other.x < self.right()
            && self.y < other.bottom()
            && other.y < self.bottom()
    }

    /// The smallest box holding both.
    pub fn union(&self, other: &Rect) -> Rect {
        let x = self.x.min(other.x);
        let y = self.y.min(other.y);
        Rect {
            x,
            y,
            width: self.right().max(other.right()) - x,
            height: self.bottom().max(other.bottom()) - y,
        }
    }

    /// The smallest box holding all of `rects`, if there are any.
    pub fn around(rects: impl IntoIterator<Item = Rect>) -> Option<Rect> {
        rects.into_iter().reduce(|a, b| a.union(&b))
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Placed {
    pub nodes: Vec<Rect>,
    /// Each frame's box in each column it has boxes in: (frame, box).
    pub frames: Vec<(usize, Rect)>,
}

impl Placed {
    /// Where calls come into box `ix`.
    pub fn in_pin(&self, ix: usize) -> (f32, f32) {
        let rect = self.nodes[ix];
        (rect.x, rect.y + HEADER_HEIGHT / 2.)
    }

    /// Where box `ix`'s calls leave it.
    pub fn out_pin(&self, ix: usize) -> (f32, f32) {
        let rect = self.nodes[ix];
        (rect.right(), rect.y + HEADER_HEIGHT / 2.)
    }

    /// Box `ix` moved to `(x, y)`.
    pub fn put(&mut self, ix: usize, (x, y): (f32, f32)) {
        self.nodes[ix].x = x;
        self.nodes[ix].y = y;
    }

    /// Everything moved by `(dx, dy)`.
    pub fn shift(&mut self, (dx, dy): (f32, f32)) {
        for rect in self.nodes.iter_mut().chain(self.frames.iter_mut().map(|(_, r)| r)) {
            rect.x += dx;
            rect.y += dy;
        }
    }

    /// The box round everything placed.
    pub fn bounds(&self) -> Option<Rect> {
        Rect::around(
            self.nodes
                .iter()
                .copied()
                .chain(self.frames.iter().map(|(_, r)| *r)),
        )
    }

    /// Each frame's box around its boxes as they are now, in the columns
    /// it was laid out in.
    pub fn refit(&mut self, plan: &Plan) {
        let mut frames: Vec<(usize, Rect)> = Vec::new();
        for (ix, node) in plan.nodes.iter().enumerate() {
            let Some(frame) = node.frame else {
                continue;
            };
            let rect = self.nodes[ix];
            // The box of this frame nearest it, else a new one.
            let near = frames.iter_mut().find(|(f, r)| {
                *f == frame && (r.x + FRAME_PAD - rect.x).abs() < NODE_WIDTH + COLUMN_GAP / 2.
            });
            let around = Rect {
                x: rect.x - FRAME_PAD,
                y: rect.y - FRAME_PAD - FRAME_TITLE,
                width: rect.width + 2. * FRAME_PAD,
                height: rect.height + 2. * FRAME_PAD + FRAME_TITLE,
            };
            match near {
                Some((_, r)) => *r = r.union(&around),
                None => frames.push((frame, around)),
            }
        }
        self.frames = frames;
    }
}

/// The rows below a box's title: its signature or detail, its badges.
pub fn rows(node: &Node) -> usize {
    usize::from(!node.detail.is_empty()) + usize::from(!node.badges.is_empty())
}

pub fn node_height(node: &Node) -> f32 {
    match rows(node) {
        0 => HEADER_HEIGHT,
        rows => HEADER_HEIGHT + rows as f32 * ROW_HEIGHT + 2. * PADDING,
    }
}

pub fn node_width(node: &Node) -> f32 {
    match node.item {
        Item::External(_) | Item::More { .. } | Item::Gone(_) => 200.,
        Item::Function(_) => NODE_WIDTH,
    }
}

pub fn layout(plan: &Plan) -> Placed {
    let mut columns: Vec<i32> = plan.nodes.iter().map(|node| node.column).collect();
    columns.sort();
    columns.dedup();
    let mut nodes = vec![Rect::default(); plan.nodes.len()];
    let column_width = NODE_WIDTH + 2. * FRAME_PAD + COLUMN_GAP;
    // Each column's groups (a frame's boxes together), packed into as many
    // side-by-side stacks as it takes to keep them short enough.
    let mut stacks: Vec<(Vec<Vec<Slot>>, f32)> = Vec::new();
    for &column in &columns {
        let mut groups: Vec<(Option<usize>, Vec<usize>)> = Vec::new();
        for ix in (0..plan.nodes.len()).filter(|&ix| plan.nodes[ix].column == column) {
            let frame = plan.nodes[ix].frame;
            match groups
                .iter_mut()
                .find(|(f, _)| frame.is_some() && *f == frame)
            {
                Some((_, list)) => list.push(ix),
                None => groups.push((frame, vec![ix])),
            }
        }
        let height = |(frame, list): &(Option<usize>, Vec<usize>)| {
            let boxes: f32 = list
                .iter()
                .map(|&ix| node_height(&plan.nodes[ix]))
                .sum::<f32>()
                + NODE_GAP * list.len().saturating_sub(1) as f32;
            boxes
                + if frame.is_some() {
                    FRAME_TITLE + 2. * FRAME_PAD
                } else {
                    0.
                }
                + GROUP_GAP
        };
        let total: f32 = groups.iter().map(height).sum();
        let count = (total / MAX_COLUMN).ceil().max(1.) as usize;
        let target = total / count as f32;
        let mut packed: Vec<Vec<(Option<usize>, Vec<usize>)>> = vec![Vec::new()];
        let mut filled = 0.;
        for group in groups {
            let h = height(&group);
            if filled > 0. && filled + h / 2. > target && packed.len() < count {
                packed.push(Vec::new());
                filled = 0.;
            }
            filled += h;
            packed.last_mut().expect("one stack").push(group);
        }
        let mut tallest: f32 = 0.;
        let mut flat = Vec::new();
        for stack in &packed {
            let mut slots = Vec::new();
            let mut h: f32 = 0.;
            for group in stack {
                h += height(group);
                slots.push(Slot::Group);
                if group.0.is_some() {
                    slots.push(Slot::Framed);
                }
                slots.extend(group.1.iter().map(|&ix| Slot::Node(ix)));
            }
            tallest = tallest.max(h);
            flat.push(slots);
        }
        stacks.push((flat, tallest - GROUP_GAP));
    }
    let tallest = stacks.iter().map(|(_, h)| *h).fold(0., f32::max);
    let mut x_at = 0.;
    for (column_stacks, height) in &stacks {
        let shift = (tallest - height) / 2.;
        for slots in column_stacks {
            let x = x_at + FRAME_PAD;
            let mut y = shift;
            let mut in_frame = false;
            let mut first = true;
            for slot in slots {
                match *slot {
                    Slot::Group => {
                        if in_frame {
                            y += FRAME_PAD;
                        }
                        if !first {
                            y += GROUP_GAP;
                        }
                        in_frame = false;
                        first = true;
                    }
                    Slot::Framed => {
                        in_frame = true;
                        y += FRAME_TITLE + FRAME_PAD;
                    }
                    Slot::Node(ix) => {
                        if !first {
                            y += NODE_GAP;
                        }
                        first = false;
                        let node = &plan.nodes[ix];
                        nodes[ix] = Rect {
                            x,
                            y,
                            width: node_width(node),
                            height: node_height(node),
                        };
                        y += nodes[ix].height;
                    }
                }
            }
            x_at += column_width;
        }
    }
    let mut placed = Placed {
        nodes,
        frames: Vec::new(),
    };
    placed.refit(plan);
    placed
}

/// A column's stack, top to bottom: where each group starts, where it's
/// in a frame, and its boxes.
enum Slot {
    Group,
    Framed,
    Node(usize),
}

#[cfg(test)]
mod tests {
    use super::super::index::Project;
    use super::super::index::tests::{fixture, id};
    use super::super::plan::{Opened, neighbourhood};
    use super::*;

    #[test]
    fn callers_go_left_and_callees_right_without_overlapping() {
        let dir = fixture();
        let project = Project::read(dir.path());
        let plan = neighbourhood(
            &project,
            id(&project, "UserService.find"),
            &Opened::default(),
        );
        let placed = layout(&plan);
        let x = |title: &str| {
            placed.nodes[plan.nodes.iter().position(|n| n.title == title).unwrap()].x
        };
        assert!(x("BugService.create") < x("UserService.find"));
        assert!(x("UserService.find") < x("UserService.lookup"));
        for (a, ra) in placed.nodes.iter().enumerate() {
            for rb in &placed.nodes[a + 1..] {
                assert!(!ra.overlaps(rb), "{ra:?} overlaps {rb:?}");
            }
        }
        // Each box inside its frame.
        for (ix, node) in plan.nodes.iter().enumerate() {
            if let Some(frame) = node.frame {
                let inside = placed
                    .frames
                    .iter()
                    .any(|(f, r)| *f == frame && r.union(&placed.nodes[ix]) == *r);
                assert!(inside, "{} outside its frame", node.title);
            }
        }
    }
}
