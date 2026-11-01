//! Drawing the Domain level: each domain a big region in its own colour,
//! its name above it in that colour, never cut short, and inside at the
//! top how much it holds, its full purpose and what it offers people;
//! Shared and
//! Unsorted greyer, apart. Jig's drafts are dashed and colourless under a
//! banner asking to index them; a proposal is outlined in the agent's
//! colour with Keep and Discard. Each link carries its flows as a label.

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::Input;
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{ActiveTheme as _, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use jig_commands::surface::{agent_accent, agent_icon};

use super::domain::{self, Field};
use super::draw::{Lit, View, blend, theme};
use super::input::Drag;
use super::scene::{Arrow, RegionBox};
use super::{Level, MapView};
use crate::domain::domains::actions::Action;
use crate::domain::domains::level::RegionKind;
mod controls;
mod region;

/// A region's name, above it: at most this big on screen.
const NAME_SIZE: f32 = 16.;
const QUIET_NAME_SIZE: f32 = 13.;
/// It shrinks to fit the room above its region down to this, then wraps.
const SMALLEST_NAME: f32 = 11.;

/// A region's name as drawn above it, on screen: its size, its lines
/// (never cut short) and how much room they take.
#[derive(Clone, Debug, PartialEq)]
pub struct NameLabel {
    pub size: f32,
    pub lines: Vec<String>,
    pub width: f32,
    pub height: f32,
}

/// Region `name` laid out to fit `room` points across: at its size if it
/// fits, else smaller, else on more lines.
pub fn name_label(name: &str, quiet: bool, room: f32) -> NameLabel {
    let most = if quiet { QUIET_NAME_SIZE } else { NAME_SIZE };
    let width_at = |text: &str, size: f32| text.chars().count() as f32 * size * 0.6;
    let size = (most * room / width_at(name, most).max(1.)).clamp(SMALLEST_NAME, most);
    let mut lines: Vec<String> = Vec::new();
    for word in name.split_whitespace() {
        match lines.last_mut() {
            Some(line) if width_at(&format!("{line} {word}"), size) <= room => {
                line.push(' ');
                line.push_str(word);
            }
            _ => lines.push(word.to_string()),
        }
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    let width = lines
        .iter()
        .map(|line| width_at(line, size))
        .fold(0., f32::max)
        + 8.;
    let height = lines.len() as f32 * (size * 1.25) + 6.;
    NameLabel {
        size,
        lines,
        width,
        height,
    }
}

/// The room a region `width` points wide on screen has for its name: its
/// width and most of the gap beside it, at `zoom`.
pub fn name_room(width: f32, zoom: f32) -> f32 {
    width + crate::domain::site::REGION_GAP * zoom * 0.8 - 8.
}

impl MapView {
    /// Over the canvas at Domain: the banner, the selected domain's
    /// actions, a name or purpose being edited, and what's being dragged.
    pub(super) fn render_domain_overlay(&self, cx: &Context<Self>) -> Vec<AnyElement> {
        let mut out = Vec::new();
        out.extend(self.render_banner(cx));
        out.extend(self.render_region_actions(cx));
        out.extend(self.render_edit_field(cx));
        out.extend(self.render_carried(cx));
        out
    }
}

/// The arrow a flow going forward (from where the link leaves) or back
/// reads with, for a link running `dx`, `dy`.
pub(super) fn glyph(forward: bool, dx: f32, dy: f32) -> &'static str {
    let (dx, dy) = if forward { (dx, dy) } else { (-dx, -dy) };
    if dx.abs() >= dy.abs() {
        if dx >= 0. { "→" } else { "←" }
    } else if dy >= 0. {
        "↓"
    } else {
        "↑"
    }
}
