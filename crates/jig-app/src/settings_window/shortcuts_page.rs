//! The Shortcuts page: every shortcut Settings can change, by group. Click
//! one's keys, then press the new ones; Esc keeps the old, ⌫ leaves it
//! without one.

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::group_box::GroupBoxVariant;
use gpui_kit::component::setting::{SettingField, SettingGroup, SettingItem, SettingPage};
use gpui_kit::component::{ActiveTheme as _, Icon, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::SettingsWindow;
use crate::settings;
use crate::shortcuts::{self, ALL, Group, Shortcut};

const KEYBOARD: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect x="2" y="5" width="20" height="14" rx="2"/><path d="M6 9h.01"/><path d="M10 9h.01"/><path d="M14 9h.01"/><path d="M18 9h.01"/><path d="M6 13h.01"/><path d="M18 13h.01"/><path d="M10 13h4"/><path d="M7 16h10"/></svg>"#;

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
    pub(super) fn shortcuts_page(&self, cx: &Context<Self>) -> SettingPage {
        let changed = settings::get(cx).shortcuts;
        let this = cx.entity().downgrade();
        let reset_all = {
            let any_changed = !changed.is_empty();
            SettingItem::render(move |_, _, cx| {
                v_flex()
                    .gap_2()
                    .child(
                        div()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child(
                                "Click a shortcut, then press the keys you want. Esc keeps \
                                 it as it was; ⌫ leaves the command without one.",
                            ),
                    )
                    .when(any_changed, |this| {
                        this.child(
                            h_flex().child(
                                Button::new("reset-shortcuts")
                                    .label("Reset All")
                                    .small()
                                    .outline()
                                    .on_click(|_, _, cx| {
                                        settings::update(cx, |s| s.shortcuts.clear())
                                    }),
                            ),
                        )
                    })
            })
            .keywords(["keyboard", "keys", "keymap", "bindings", "reset"])
        };
        let mut page = SettingPage::new("Shortcuts")
            .icon(Icon::default().data(KEYBOARD))
            .group(
                SettingGroup::new()
                    .variant(GroupBoxVariant::Normal)
                    .item(reset_all),
            );
        for group in Group::ALL {
            let items = ALL
                .iter()
                .filter(|shortcut| shortcut.group == group)
                .map(|shortcut| {
                    let clashes = shortcuts::clashes(shortcut, &changed);
                    let mut item = SettingItem::new(
                        shortcut.label,
                        SettingField::render({
                            let this = this.clone();
                            move |_, _, cx| keys_field(shortcut, this.clone(), cx)
                        }),
                    )
                    .keywords([shortcut.id]);
                    if !clashes.is_empty() {
                        item = item.description(format!("Also {}.", clashes.join(", ")));
                    }
                    item
                });
            page = page.group(SettingGroup::new().title(group.label()).items(items));
        }
        page
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

/// The shortcut's keys as a button that records new ones, and Reset once
/// it's been changed.
fn keys_field(
    shortcut: &'static Shortcut,
    this: WeakEntity<SettingsWindow>,
    cx: &mut App,
) -> AnyElement {
    let recording = this.upgrade().is_some_and(|this| {
        this.read(cx)
            .recording
            .as_ref()
            .is_some_and(|r| r.id == shortcut.id)
    });
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
                    .label("Reset")
                    .small()
                    .ghost()
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
