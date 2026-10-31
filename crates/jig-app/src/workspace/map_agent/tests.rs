use serde_json::json;

use jig_ai::agent::Turn;
use jig_commands::ChatEntry;

use super::transcript;

#[test]
fn an_earlier_conversations_drawings_come_back() {
    let (entries, drawings) = transcript(vec![
        Turn::User("How does it work?".into()),
        Turn::Drew {
            tool: "map_trace".into(),
            input: json!({"title": "Flow", "steps": [{"place": "a.rs", "label": "Start"}]}),
        },
        Turn::Drew {
            tool: "map_clear".into(),
            input: json!({}),
        },
        Turn::Drew {
            tool: "map_highlight".into(),
            input: json!({"title": "These", "places": ["b.rs"]}),
        },
        Turn::Agent("Like [this](a.rs).".into()),
    ]);
    assert_eq!(drawings.len(), 2);
    assert!(!drawings[0].shown, "cleared");
    assert!(drawings[1].shown);
    assert!(matches!(
        &entries[1],
        ChatEntry::Drawing { shown: false, summary, .. } if summary == "Trace: Flow · 1 step"
    ));
    assert_eq!(entries.len(), 4);
}
