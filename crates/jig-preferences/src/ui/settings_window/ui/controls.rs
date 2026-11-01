//! The controls rows hold: buttons, switches, dropdowns and text and
//! number fields.

use super::*;

/// A plain button, the kind most rows use.
pub fn button(id: impl Into<SharedString>, label: impl Into<SharedString>) -> Button {
    Button::new(ElementId::Name(id.into()))
        .label(label.into())
        .small()
        .outline()
}

pub fn switch(
    id: impl Into<SharedString>,
    get: impl Fn(&App) -> bool + 'static,
    set: impl Fn(bool, &mut App) + 'static,
) -> impl Fn(&mut Window, &mut App) -> AnyElement + 'static {
    let id = id.into();
    let set = Rc::new(set);
    move |_, cx| {
        let set = set.clone();
        Switch::new(ElementId::Name(format!("switch-{id}").into()))
            .checked(get(cx))
            .small()
            .on_click(move |on: &bool, _, cx| set(*on, cx))
            .into_any_element()
    }
}

/// A pop-up button listing `options` as (value, label), the chosen one
/// checked.
pub fn dropdown(
    id: impl Into<SharedString>,
    options: Vec<(SharedString, SharedString)>,
    get: impl Fn(&App) -> SharedString + 'static,
    set: impl Fn(SharedString, &mut App) + 'static,
) -> impl Fn(&mut Window, &mut App) -> AnyElement + 'static {
    let id = id.into();
    let options = Rc::new(options);
    let set = Rc::new(set);
    move |_, cx| {
        let current = get(cx);
        let label = options
            .iter()
            .find(|(value, _)| *value == current)
            .map(|(_, label)| label.clone())
            .unwrap_or_else(|| current.clone());
        let options = options.clone();
        let set = set.clone();
        let scrollable = options.len() > 12;
        Button::new(ElementId::Name(format!("dropdown-{id}").into()))
            .label(label)
            .dropdown_caret(true)
            .small()
            .outline()
            .max_w(px(FIELD_WIDTH))
            .dropdown_menu_with_anchor(Anchor::TopRight, move |menu, _, _| {
                options
                    .iter()
                    .fold(menu, |menu, (value, label)| {
                        let set = set.clone();
                        let value = value.clone();
                        menu.item(
                            PopupMenuItem::new(label.clone())
                                .checked(value == current)
                                .on_click(move |_, _, cx| set(value.clone(), cx)),
                        )
                    })
                    .scrollable(scrollable)
            })
            .into_any_element()
    }
}

/// An input that keeps its state across frames, saving as it's typed and
/// showing changes made elsewhere.
struct TextState {
    input: Entity<InputState>,
    _changes: Subscription,
}

pub fn text_input(
    id: impl Into<SharedString>,
    get: impl Fn(&App) -> SharedString + 'static,
    set: impl Fn(SharedString, &mut App) + 'static,
) -> impl Fn(&mut Window, &mut App) -> AnyElement + 'static {
    let id = id.into();
    let set = Rc::new(set);
    move |window, cx| {
        let value = get(cx);
        let set = set.clone();
        let state = window.use_keyed_state(
            SharedString::from(format!("text-{id}")),
            cx,
            |window, cx| {
                let input = cx.new(|cx| InputState::new(window, cx).default_value(value.clone()));
                let _changes = cx.subscribe(&input, move |_, input, event, cx| {
                    if let InputEvent::Change = event {
                        set(input.read(cx).value(), cx);
                    }
                });
                TextState { input, _changes }
            },
        );
        let input = state.read(cx).input.clone();
        if input.read(cx).value() != value {
            input.update(cx, |input, cx| input.set_value(value, window, cx));
        }
        Input::new(&input)
            .small()
            .w(px(FIELD_WIDTH))
            .into_any_element()
    }
}

/// A number field with − and + buttons; values outside `min..=max` aren't
/// saved.
pub fn number_input(
    id: impl Into<SharedString>,
    (min, max, step): (f64, f64, f64),
    get: impl Fn(&App) -> f64 + 'static,
    set: impl Fn(f64, &mut App) + 'static,
) -> impl Fn(&mut Window, &mut App) -> AnyElement + 'static {
    let id = id.into();
    let set = Rc::new(set);
    move |window, cx| {
        let value = get(cx);
        let set = set.clone();
        let state = window.use_keyed_state(
            SharedString::from(format!("number-{id}")),
            cx,
            |window, cx| {
                let input = cx.new(|cx| {
                    InputState::new(window, cx)
                        .default_value(value.to_string())
                        .step(step)
                        .min(min)
                        .max(max)
                });
                let _changes = cx.subscribe(&input, move |_, input, event, cx| {
                    if let InputEvent::Change = event
                        && let Ok(number) = input.read(cx).value().parse::<f64>()
                        && (min..=max).contains(&number)
                    {
                        set(number, cx);
                    }
                });
                TextState { input, _changes }
            },
        );
        let input = state.read(cx).input.clone();
        let shown = input.read(cx).value().parse::<f64>().ok();
        if shown != Some(value) && !input.read(cx).focus_handle(cx).is_focused(window) {
            input.update(cx, |input, cx| {
                input.set_value(SharedString::from(value.to_string()), window, cx)
            });
        }
        NumberInput::new(&input)
            .small()
            .w(px(112.))
            .into_any_element()
    }
}
