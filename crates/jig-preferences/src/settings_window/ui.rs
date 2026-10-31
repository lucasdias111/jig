//! The pieces every Settings page is made of, styled like the settings of a
//! Mac app: titled sections, each a rounded card of rows divided by
//! hairlines, a label (and a line about it) on the left and its control on
//! the right. Pages describe their rows as data, so search can look through
//! all of them and only the page showing is drawn.

use std::rc::Rc;

use gpui_kit::component::button::Button;
use gpui_kit::component::input::{Input, InputEvent, InputState, NumberInput};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::switch::Switch;
use gpui_kit::component::{ActiveTheme as _, Icon, IconName, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

/// Label and control text.
pub const TEXT: f32 = 13.;
/// Descriptions, footers and other secondary text.
pub const SMALL: f32 = 12.;
const RADIUS: f32 = 8.;
/// Room the controls get beside a label.
const FIELD_WIDTH: f32 = 220.;

/// Draws a row's control, or anything else drawn fresh each frame.
pub type Draw = Rc<dyn Fn(&mut Window, &mut App) -> AnyElement>;

/// A titled card of rows, with an optional line under it.
#[derive(Clone, Default)]
pub struct Section {
    pub title: Option<SharedString>,
    pub footer: Option<SharedString>,
    /// Beside the title, e.g. a Reset All button.
    pub accessory: Option<Draw>,
    pub rows: Vec<Row>,
}

impl Section {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn titled(title: impl Into<SharedString>) -> Self {
        Self {
            title: Some(title.into()),
            ..Self::default()
        }
    }

    pub fn footer(mut self, footer: impl Into<SharedString>) -> Self {
        self.footer = Some(footer.into());
        self
    }

    pub fn accessory<E: IntoElement>(
        mut self,
        draw: impl Fn(&mut Window, &mut App) -> E + 'static,
    ) -> Self {
        self.accessory = Some(Rc::new(move |window, cx| {
            draw(window, cx).into_any_element()
        }));
        self
    }

    pub fn row(mut self, row: Row) -> Self {
        self.rows.push(row);
        self
    }

    pub fn rows(mut self, rows: impl IntoIterator<Item = Row>) -> Self {
        self.rows.extend(rows);
        self
    }

    /// Just the rows that match `query`, or `None` when none do.
    pub fn matching(&self, query: &str) -> Option<Section> {
        let rows: Vec<Row> = self
            .rows
            .iter()
            .filter(|row| row.matches(query))
            .cloned()
            .collect();
        (!rows.is_empty()).then(|| Section {
            rows,
            footer: None,
            ..self.clone()
        })
    }
}

/// Where a row leads when clicked, for rows that open a page of their own.
pub type Open = Rc<dyn Fn(&mut Window, &mut App)>;

/// One setting.
#[derive(Clone)]
pub struct Row {
    pub id: SharedString,
    pub label: SharedString,
    pub description: Option<SharedString>,
    /// The description is a problem, shown in red.
    pub warning: bool,
    pub keywords: Vec<SharedString>,
    pub control: Option<Draw>,
    /// Shown dimmed before the chevron of a row that opens a page.
    pub value: Option<SharedString>,
    pub open: Option<Open>,
}

impl Row {
    pub fn new(id: impl Into<SharedString>, label: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            description: None,
            warning: false,
            keywords: Vec::new(),
            control: None,
            value: None,
            open: None,
        }
    }

    /// A row of text alone, e.g. why a list is empty.
    pub fn note(id: impl Into<SharedString>, text: impl Into<SharedString>) -> Self {
        Self::new(id, "").description(text)
    }

    pub fn description(mut self, description: impl Into<SharedString>) -> Self {
        self.description = Some(description.into());
        self
    }

    pub fn description_opt(mut self, description: Option<impl Into<SharedString>>) -> Self {
        self.description = description.map(Into::into);
        self
    }

    pub fn warning(mut self, warning: bool) -> Self {
        self.warning = warning;
        self
    }

    pub fn keywords<S: Into<SharedString>>(
        mut self,
        keywords: impl IntoIterator<Item = S>,
    ) -> Self {
        self.keywords.extend(keywords.into_iter().map(Into::into));
        self
    }

    pub fn control<E: IntoElement>(
        mut self,
        draw: impl Fn(&mut Window, &mut App) -> E + 'static,
    ) -> Self {
        self.control = Some(Rc::new(move |window, cx| {
            draw(window, cx).into_any_element()
        }));
        self
    }

    pub fn value(mut self, value: impl Into<SharedString>) -> Self {
        self.value = Some(value.into());
        self
    }

    pub fn opens(mut self, open: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.open = Some(Rc::new(open));
        self
    }

    pub fn matches(&self, query: &str) -> bool {
        let query = query.trim().to_lowercase();
        let has = |text: &str| text.to_lowercase().contains(&query);
        has(&self.label)
            || self.description.as_deref().is_some_and(has)
            || self.keywords.iter().any(|keyword| has(keyword))
    }
}

/// The card behind a section's rows: a shade off the window, so it reads
/// as a group without a heavy outline.
pub fn card_background(cx: &App) -> Hsla {
    let background = cx.theme().background;
    let lift = if cx.theme().is_dark() { 0.035 } else { 0.022 };
    Hsla {
        l: (background.l + lift).min(1.),
        ..background
    }
}

/// A hairline, for card edges and between rows.
pub fn hairline(cx: &App) -> Hsla {
    if cx.theme().is_dark() {
        hsla(0., 0., 1., 0.08)
    } else {
        hsla(0., 0., 0., 0.08)
    }
}

pub fn render_section(
    key: impl Into<SharedString>,
    section: &Section,
    window: &mut Window,
    cx: &mut App,
) -> Div {
    let key = key.into();
    let heading = (section.title.is_some() || section.accessory.is_some()).then(|| {
        h_flex()
            .min_h(px(24.))
            .px_1()
            .justify_between()
            .items_end()
            .child(
                div()
                    .text_size(px(TEXT))
                    .font_weight(FontWeight::SEMIBOLD)
                    .children(section.title.clone()),
            )
            .children(section.accessory.as_ref().map(|draw| draw(window, cx)))
    });
    let border = hairline(cx);
    let mut card = v_flex()
        .bg(card_background(cx))
        .border_1()
        .border_color(border)
        .rounded(px(RADIUS));
    for (ix, row) in section.rows.iter().enumerate() {
        card = card.child(
            div()
                .when(ix > 0, |this| {
                    // Divided from the row above, inset like a Mac list.
                    this.child(div().ml_3().h(px(1.)).bg(border))
                })
                .child(render_row(&key, row, ix, window, cx)),
        );
    }
    v_flex()
        .gap_1p5()
        .children(heading)
        .when(!section.rows.is_empty(), |this| this.child(card))
        .when_some(section.footer.clone(), |this, footer| {
            this.child(
                div()
                    .px_1()
                    .text_size(px(SMALL))
                    .text_color(cx.theme().muted_foreground)
                    .child(footer),
            )
        })
}

fn render_row(
    key: &SharedString,
    row: &Row,
    ix: usize,
    window: &mut Window,
    cx: &mut App,
) -> Stateful<Div> {
    let theme = cx.theme();
    let muted = theme.muted_foreground;
    let description_color = if row.warning { theme.danger } else { muted };
    let hover = if theme.is_dark() {
        hsla(0., 0., 1., 0.04)
    } else {
        hsla(0., 0., 0., 0.03)
    };
    let text = v_flex()
        .flex_1()
        .min_w_0()
        .gap_0p5()
        .when(!row.label.is_empty(), |this| {
            this.child(div().text_size(px(TEXT)).child(row.label.clone()))
        })
        .when_some(row.description.clone(), |this, description| {
            this.child(
                div()
                    .text_size(px(SMALL))
                    .line_height(relative(1.35))
                    .text_color(description_color)
                    .child(description),
            )
        });
    let trailing = h_flex()
        .flex_shrink_0()
        .gap_2()
        .items_center()
        .children(row.control.as_ref().map(|draw| draw(window, cx)))
        .when(row.open.is_some(), |this| {
            this.children(
                row.value
                    .clone()
                    .map(|value| div().text_size(px(TEXT)).text_color(muted).child(value)),
            )
            .child(
                Icon::new(IconName::ChevronRight)
                    .size(px(14.))
                    .text_color(muted),
            )
        });
    h_flex()
        .id(SharedString::from(format!("{key}-{ix}-{}", row.id)))
        .min_h(px(40.))
        .px_3()
        .py_2()
        .gap_4()
        .items_center()
        .child(text)
        .child(trailing)
        .when_some(row.open.clone(), |this, open| {
            this.cursor_pointer()
                .hover(move |style| style.bg(hover))
                .on_click(move |_, window, cx| open(window, cx))
        })
}

/// A page's sections, one under the other.
pub fn render_sections(key: &str, sections: &[Section], window: &mut Window, cx: &mut App) -> Div {
    v_flex().gap_6().children(
        sections
            .iter()
            .enumerate()
            .map(|(ix, section)| render_section(format!("{key}-{ix}"), section, window, cx)),
    )
}

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
