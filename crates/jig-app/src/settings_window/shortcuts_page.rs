//! The Keyboard page: every shortcut Settings can change, by group. Click
//! one's keys, then press the new ones; Esc keeps the old, ⌫ leaves it
//! without one.

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{IconName, Sizable as _, h_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::SettingsWindow;
use super::ui::{self, Row, Section};
use crate::settings;
use crate::shortcuts::{self, ALL, Group, Shortcut};

/// Keys that only modify another; recording waits past them.
const MODIFIERS: &[&str] = &[
    "shift", "control", "ctrl", "alt", "option", "cmd", "command", "platform", "super", "win",
    "fn", "function",
];

/// The shortcut whose keys are being pressed.
pub(super) struct Recording {
    id: &'static str,
    _intercept: Subscription,
}

impl SettingsWindow {
    pub(super) fn shortcuts_sections(&self, cx: &Context<Self>) -> Vec<Section> {
        let changed = settings::get(cx).shortcuts;
        let any_changed = !changed.is_empty();
        let recording = self.recording.as_ref().map(|recording| recording.id);
        let this = cx.entity().downgrade();
        Group::ALL
            .into_iter()
            .enumerate()
            .map(|(ix, group)| {
                let rows = ALL
                    .iter()
                    .filter(|shortcut| shortcut.group == group)
                    .map(|shortcut| {
                        let clashes = shortcuts::clashes(shortcut, &changed);
                        let this = this.clone();
                        Row::new(shortcut.id, shortcut.label)
                            .description_opt(
                                (!clashes.is_empty())
                                    .then(|| format!("Also {}.", clashes.join(", "))),
                            )
                            .warning(!clashes.is_empty())
                            .keywords([shortcut.id, "shortcut", "key", "keyboard"])
                            .control(move |_, cx| {
                                keys_field(
                                    shortcut,
                                    recording == Some(shortcut.id),
                                    this.clone(),
                                    cx,
                                )
                            })
                    });
                let mut section = Section::titled(group.label()).rows(rows);
                if ix == 0 && any_changed {
                    section = section.accessory(|_, _| {
                        ui::button("reset-shortcuts", "Reset All")
                            .on_click(|_, _, cx| settings::update(cx, |s| s.shortcuts.clear()))
                    });
                }
                if ix + 1 == Group::ALL.len() {
                    section = section.footer(
                        "Click a shortcut, then press the keys you want. Esc keeps it as \
                         it was; ⌫ leaves the command without one.",
                    );
                }
                section
            })
            .collect()
    }

    pub(super) fn record(&mut self, id: &'static str, window: &mut Window, cx: &mut Context<Self>) {
        if self
            .recording
            .as_ref()
            .is_some_and(|recording| recording.id == id)
        {
            self.recording = None;
            cx.notify();
            return;
        }
        let handle = window.window_handle();
        let this = cx.entity().downgrade();
        // Ahead of the keymap, so the keys pressed don't also run whatever
        // they're bound to now.
        let intercept = cx.intercept_keystrokes(move |event, window, cx| {
            if window.window_handle() != handle || MODIFIERS.contains(&event.keystroke.key.as_str())
            {
                return;
            }
            cx.stop_propagation();
            let keystroke = event.keystroke.clone();
            this.update(cx, |this, cx| this.recorded(&keystroke, cx))
                .ok();
        });
        self.recording = Some(Recording {
            id,
            _intercept: intercept,
        });
        cx.notify();
    }

    fn recorded(&mut self, keystroke: &Keystroke, cx: &mut Context<Self>) {
        let Some(recording) = self.recording.take() else {
            return;
        };
        cx.notify();
        let plain = !keystroke.modifiers.modified();
        let keys = match keystroke.key.as_str() {
            "escape" if plain => return,
            "backspace" | "delete" if plain => String::new(),
            _ => keystroke.unparse(),
        };
        let Some(shortcut) = ALL.iter().find(|shortcut| shortcut.id == recording.id) else {
            return;
        };
        let is_default = shortcut.keys.len() == 1
            && Keystroke::parse(shortcut.keys[0]).is_ok_and(|default| default.unparse() == keys);
        settings::update(cx, |s| {
            if is_default {
                s.shortcuts.remove(shortcut.id);
            } else {
                s.shortcuts.insert(shortcut.id.to_string(), keys);
            }
        });
    }
}

/// The shortcut's keys as a button that records new ones, and a reset
/// button once it's been changed.
fn keys_field(
    shortcut: &'static Shortcut,
    recording: bool,
    this: WeakEntity<SettingsWindow>,
    cx: &mut App,
) -> AnyElement {
    let changed = settings::get(cx).shortcuts;
    let keys = shortcut.keys_in(&changed);
    let label = if recording {
        "Press keys…".to_string()
    } else if keys.is_empty() {
        "None".to_string()
    } else {
        keys.iter()
            .map(|keys| shortcuts::display(keys))
            .collect::<Vec<_>>()
            .join("  or  ")
    };
    let keys_button = Button::new(SharedString::from(format!("keys-{}", shortcut.id)))
        .label(label)
        .small()
        .min_w(px(72.))
        .map(|button| {
            if recording {
                button.primary()
            } else {
                button.outline()
            }
        })
        .on_click({
            let this = this.clone();
            move |_, window, cx| {
                this.update(cx, |this, cx| this.record(shortcut.id, window, cx))
                    .ok();
            }
        });
    h_flex()
        .gap_1()
        .when(changed.contains_key(shortcut.id), |row| {
            row.child(
                Button::new(SharedString::from(format!("reset-{}", shortcut.id)))
                    .icon(IconName::Undo2)
                    .ghost()
                    .xsmall()
                    .tooltip("Back to the default")
                    .on_click(move |_, _, cx| {
                        settings::update(cx, |s| {
                            s.shortcuts.remove(shortcut.id);
                        })
                    }),
            )
        })
        .child(keys_button)
        .into_any_element()
}
