use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    AppContext, Bounds, Context, Entity, IntoElement, ParentElement, Point, Render, Styled,
    Subscription, TestAppContext, Window, WindowBounds, WindowOptions, div, px, size,
};

use super::{CommandPalette, PaletteEvent, Row};

struct Host {
    palette: Entity<CommandPalette>,
    /// What it said: "run …", "resume ses_1", "command review main".
    events: Vec<String>,
    _subscription: Subscription,
}

impl Render for Host {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full().child(self.palette.clone())
    }
}

/// The palette with two earlier conversations and two commands listed.
fn open(cx: &mut TestAppContext) -> (gpui_kit::AnyWindowHandle, Entity<Host>) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        // Jig binds it from its shortcuts table.
        cx.bind_keys([gpui_kit::KeyBinding::new(
            "alt-m",
            super::PickModel,
            Some(super::CONTEXT),
        )]);
    });
    let (window, host) = cx.update(|cx| {
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds {
                origin: Point::default(),
                size: size(px(600.), px(400.)),
            })),
            ..Default::default()
        };
        gpui_kit::open_window(options, cx, |window, cx| {
            cx.new(|cx| {
                let palette = cx.new(|cx| CommandPalette::new(true, window, cx));
                let subscription =
                    cx.subscribe(&palette, |host: &mut Host, _, event: &PaletteEvent, _| {
                        host.events.push(match event {
                            PaletteEvent::Run(text) => format!("run {text}"),
                            PaletteEvent::Resume(id) => format!("resume {id}"),
                            PaletteEvent::RunAgentCommand { name, arguments } => {
                                format!("command {name} {arguments}").trim().into()
                            }
                            PaletteEvent::PickModel(choice) => {
                                format!("model {}", choice.label)
                            }
                            PaletteEvent::Dismissed => "dismissed".into(),
                        })
                    });
                Host {
                    palette,
                    events: Vec::new(),
                    _subscription: subscription,
                }
            })
        })
        .unwrap()
    });
    cx.update(|cx| {
        let palette = host.read(cx).palette.clone();
        palette.update(cx, |palette, cx| {
            palette.set_agent_catalog(
                vec![
                    super::PastConversation {
                        id: "ses_1".into(),
                        title: "Add docs to the parser".into(),
                        when: "2 h ago".into(),
                    },
                    super::PastConversation {
                        id: "ses_2".into(),
                        title: "Fix the flaky test".into(),
                        when: "yesterday".into(),
                    },
                ],
                vec![
                    super::AgentCommandInfo {
                        name: "review".into(),
                        description: "review changes".into(),
                        skill: false,
                    },
                    super::AgentCommandInfo {
                        name: "init".into(),
                        description: "guided AGENTS.md setup".into(),
                        skill: false,
                    },
                ],
                cx,
            )
        })
    });
    cx.run_until_parked();
    (window, host)
}

fn step(
    cx: &mut TestAppContext,
    window: gpui_kit::AnyWindowHandle,
    f: impl FnOnce(&mut Window, &mut gpui_kit::App),
) {
    cx.update_window(window, |_, window, cx| f(window, cx))
        .unwrap();
    cx.run_until_parked();
}

fn events(cx: &mut TestAppContext, host: &Entity<Host>) -> Vec<String> {
    cx.update(|cx| host.read(cx).events.clone())
}

fn query(cx: &mut TestAppContext, host: &Entity<Host>) -> String {
    cx.update(|cx| host.read(cx).palette.read(cx).query(cx))
}

fn rows(cx: &mut TestAppContext, host: &Entity<Host>) -> Vec<Row> {
    cx.update(|cx| host.read(cx).palette.read(cx).rows.clone())
}

#[gpui_kit::test]
fn typed_text_is_a_message_for_the_agent(cx: &mut TestAppContext) {
    let (window, host) = open(cx);
    step(cx, window, |window, cx| {
        window.render_frame(cx);
        window.input("make this async", cx);
    });
    step(cx, window, |window, cx| window.press("enter", cx));
    assert_eq!(events(cx, &host), ["run make this async"]);
}

#[gpui_kit::test]
fn escape_dismisses(cx: &mut TestAppContext) {
    let (window, host) = open(cx);
    step(cx, window, |window, cx| {
        window.render_frame(cx);
        window.press("escape", cx);
    });
    assert_eq!(events(cx, &host), ["dismissed"]);
}

#[gpui_kit::test]
fn earlier_conversations_are_listed_below_the_message(cx: &mut TestAppContext) {
    let (window, host) = open(cx);
    assert_eq!(
        rows(cx, &host),
        [Row::Conversation(0), Row::Conversation(1)]
    );
    step(cx, window, |window, cx| window.input("flaky", cx));
    assert_eq!(
        rows(cx, &host),
        [Row::Message("flaky".into()), Row::Conversation(1)]
    );
    step(cx, window, |window, cx| {
        window.press("down", cx);
        window.press("enter", cx);
    });
    assert_eq!(events(cx, &host), ["resume ses_2"]);
}

#[gpui_kit::test]
fn slash_lists_commands_and_runs_one_with_arguments(cx: &mut TestAppContext) {
    let (window, host) = open(cx);
    step(cx, window, |window, cx| window.input("/", cx));
    assert_eq!(rows(cx, &host), [Row::Command(0), Row::Command(1)]);
    step(cx, window, |window, cx| window.input("rev", cx));
    assert_eq!(rows(cx, &host), [Row::Command(0)]);
    step(cx, window, |window, cx| window.press("right", cx));
    assert_eq!(query(cx, &host), "/review");
    step(cx, window, |window, cx| window.input(" main", cx));
    step(cx, window, |window, cx| window.press("enter", cx));
    assert_eq!(events(cx, &host), ["command review main"]);
}

#[gpui_kit::test]
fn an_unknown_slash_command_goes_to_the_agent_as_text(cx: &mut TestAppContext) {
    let (window, host) = open(cx);
    step(cx, window, |window, cx| {
        window.input("/nothing like it", cx)
    });
    assert_eq!(rows(cx, &host), [Row::Message("/nothing like it".into())]);
}

#[gpui_kit::test]
fn shift_enter_starts_a_new_line(cx: &mut TestAppContext) {
    let (window, host) = open(cx);
    step(cx, window, |window, cx| window.input("rename it", cx));
    step(cx, window, |window, cx| window.press("shift-enter", cx));
    step(cx, window, |window, cx| window.input("and its tests", cx));
    assert_eq!(query(cx, &host), "rename it\nand its tests");
    assert!(events(cx, &host).is_empty());
    step(cx, window, |window, cx| window.press("enter", cx));
    assert_eq!(events(cx, &host), ["run rename it\nand its tests"]);
}

#[gpui_kit::test]
fn right_moves_the_cursor_when_there_is_nothing_to_complete(cx: &mut TestAppContext) {
    let (window, host) = open(cx);
    step(cx, window, |window, cx| window.input("/rev", cx));
    step(cx, window, |window, cx| {
        window.press("left", cx);
        window.press("right", cx);
    });
    assert_eq!(query(cx, &host), "/rev", "away from the end, → just moves");
}

#[gpui_kit::test]
fn the_chip_picks_an_agent_then_its_model(cx: &mut TestAppContext) {
    let (window, host) = open(cx);
    let choice = |agent: &str, group: &str, name: &str| super::ModelChoice {
        agent: Some(agent.into()),
        group: group.into(),
        name: name.into(),
        model: Some(name.to_lowercase()),
        label: format!("{group} · {name}"),
    };
    cx.update(|cx| {
        let palette = host.read(cx).palette.clone();
        palette.update(cx, |palette, _| {
            palette.models = super::ModelMenu {
                choices: vec![
                    choice("opencode", "OpenCode", "Haiku"),
                    choice("opencode", "OpenCode", "Sonnet"),
                    choice("claude-code", "Claude Code", "Opus"),
                ],
                current: Some(0),
            };
        })
    });
    step(cx, window, |window, cx| window.input("explain", cx));
    step(cx, window, |window, cx| window.press("alt-m", cx));
    assert_eq!(query(cx, &host), "", "the typed text is set aside");
    assert_eq!(rows(cx, &host), [Row::Group(0), Row::Group(1)]);
    // Enter on OpenCode lists its models, ⌫ goes back.
    step(cx, window, |window, cx| window.press("enter", cx));
    assert_eq!(rows(cx, &host), [Row::Model(0), Row::Model(1)]);
    step(cx, window, |window, cx| window.press("backspace", cx));
    assert_eq!(rows(cx, &host), [Row::Group(0), Row::Group(1)]);
    // A model can be typed straight away too.
    step(cx, window, |window, cx| window.input("opus", cx));
    assert_eq!(rows(cx, &host), [Row::Model(2)]);
    step(cx, window, |window, cx| window.press("enter", cx));
    assert_eq!(events(cx, &host), ["model Claude Code · Opus"]);
    assert_eq!(query(cx, &host), "explain", "and back after");
    step(cx, window, |window, cx| window.press("alt-m", cx));
    step(cx, window, |window, cx| window.press("escape", cx));
    assert!(cx.update(|cx| host.read(cx).palette.read(cx).picking.is_none()));
}
