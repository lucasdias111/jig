use super::*;
use serde_json::json;

fn labels(found: &[Completion]) -> Vec<&str> {
    found.iter().map(|c| c.item.label.as_str()).collect()
}

#[test]
fn words_nearest_first_and_not_the_one_being_typed() {
    let text = "let counter = 1;\nlet count = 2;\nlet countdown = 3;\ncou";
    let found = from_words(text, text.len());
    assert_eq!(labels(&found), ["countdown", "count", "counter"]);
    assert_eq!(found[0].replace, text.len() - 3..text.len());
    assert_eq!(found[0].new_text, "countdown");
}

#[test]
fn words_start_at_the_first_letter() {
    let text = "counter\nc";
    assert_eq!(labels(&from_words(text, text.len())), ["counter"]);
    let text = "counter\n";
    assert!(from_words(text, text.len()).is_empty());
}

#[test]
fn numbers_are_not_completed() {
    let text = "v1 = 10;\n1";
    assert!(from_words(text, text.len()).is_empty());
}

#[test]
fn words_match_any_case() {
    let text = "HttpClient\nhttp";
    assert_eq!(labels(&from_words(text, text.len())), ["HttpClient"]);
}

#[test]
fn fields_could_start_after_a_brace_or_comma() {
    let at_end = |text: &str| could_start_a_field(text, text.len());
    assert!(at_end("let p = Point {"));
    assert!(at_end("let p = Point { "));
    assert!(at_end("let p = Point {\n        "));
    assert!(at_end("let p = Point { x: 1, "));
    assert!(!at_end("let p = Point { x: 1, he"), "a name is being typed");
    assert!(!at_end("let p = Point { x: "));
    assert!(!at_end("let n = 1 "));
}

#[test]
fn only_fields_is_a_struct_literal_s_list() {
    let found = |kinds: &[(&str, u32)]| {
        let response: Vec<_> = kinds
            .iter()
            .map(|(label, kind)| json!({"label": label, "kind": kind}))
            .collect();
        from_server("", 0, Encoding::Utf8, &json!(response), "    ")
    };
    assert!(only_fields(&found(&[("x", 5), ("height", 5)])));
    assert!(only_fields(&found(&[
        ("x", 5),
        ("..Default::default()", 15)
    ])));
    assert!(!only_fields(&found(&[("x", 5), ("main", 3)])), "a block");
    assert!(!only_fields(&found(&[])));
}

#[test]
fn server_items_are_narrowed_to_what_was_typed() {
    let text = "fn main() {\n    v.pu\n}\n";
    let offset = text.find("pu").unwrap() + 2;
    // A server that returns everything and leaves narrowing to us.
    let response = json!({"isIncomplete": false, "items": [
        {"label": "len", "sortText": "1"},
        {"label": "pop", "sortText": "2"},
        {"label": "push", "sortText": "3", "textEdit": {
            "range": {"start": {"line": 1, "character": 6}, "end": {"line": 1, "character": 8}},
            "newText": "push"}},
        {"label": "Pull", "sortText": "0"},
        {"label": "map_unwrap", "sortText": "0"},
    ]});
    let found = from_server(text, offset, Encoding::Utf8, &response, "    ");
    assert_eq!(labels(&found), ["push", "Pull", "map_unwrap"]);
    assert_eq!(found[0].replace, offset - 2..offset);
    assert_eq!(found[1].replace, offset - 2..offset);
    assert_eq!(found[1].new_text, "Pull");
}

#[test]
fn server_insert_text_and_snippets_become_plain_text() {
    let text = "v.";
    let response = json!([
        {"label": "push(…)", "filterText": "push", "insertText": "push(${1:value})$0", "insertTextFormat": 2},
        {"label": "iter", "insertText": "iter()"},
    ]);
    let found = from_server(text, 2, Encoding::Utf16, &response, "    ");
    assert_eq!(
        found
            .iter()
            .map(|c| c.new_text.as_str())
            .collect::<Vec<_>>(),
        ["iter()", "push(value)"]
    );
    assert!(found.iter().all(|c| c.replace == (2..2)));
}

#[test]
fn no_answer_is_no_completions() {
    assert!(from_server("a", 1, Encoding::Utf8, &serde_json::Value::Null, "    ").is_empty());
}

#[test]
fn server_snippets_keep_their_places() {
    let response = json!([{
        "label": "push", "insertText": "push(${1:value})$0", "insertTextFormat": 2,
    }]);
    let found = from_server("v.", 2, Encoding::Utf8, &response, "    ");
    assert_eq!(found[0].new_text, "push(value)");
    assert_eq!(found[0].stops, [5..10, 11..11]);
}

#[test]
fn edits_just_before_join_and_others_are_left_out() {
    let text = "    v.le";
    let at = |line: u32, start: u32, end: u32| json!({"start": {"line": line, "character": start}, "end": {"line": line, "character": end}});
    let response = json!([
        {"label": "let", "insertTextFormat": 2,
         "textEdit": {"range": at(0, 6, 8), "newText": "let $0 = v;"},
         "additionalTextEdits": [{"range": at(0, 4, 6), "newText": ""}]},
        {"label": "lerp", "additionalTextEdits": [{"range": at(0, 0, 0), "newText": "use x::lerp;\n"}]},
    ]);
    let found = from_server(text, text.len(), Encoding::Utf8, &response, "    ");
    assert_eq!(labels(&found), ["let"]);
    assert_eq!(found[0].replace, 4..8);
    assert_eq!(found[0].new_text, "let  = v;");
    assert_eq!(found[0].stops.first(), Some(&(4..4)));
}

fn snippet(name: &str, postfix: bool, body: &str) -> Snippet {
    Snippet {
        name: name.into(),
        description: String::new(),
        languages: vec!["rust".into()],
        postfix,
        body: body.into(),
    }
}

#[test]
fn prefix_snippets_by_name_indented_to_the_line() {
    let snippets = [
        snippet("for", false, "for ${1:item} in ${2:items} {\n\t$0\n}"),
        snippet("fori", false, "for ${1:i} in 0..${2:n} {\n\t$0\n}"),
        snippet("var", true, "let ${1:$NAME} = $EXPR;"),
    ];
    let text = "fn a() {\n    fo";
    let found = from_snippets(text, text.len(), "rust", &snippets, "    ");
    assert_eq!(labels(&found), ["for", "fori"]);
    assert_eq!(found[0].new_text, "for item in items {\n        \n    }");
    assert_eq!(found[0].replace, text.len() - 2..text.len());
    assert_eq!(found[0].stops[0], 4..8);
    // One letter isn't enough to offer them.
    assert!(from_snippets("f", 1, "rust", &snippets, "    ").is_empty());
}

#[test]
fn postfix_snippets_wrap_the_expression() {
    let snippets = [
        snippet("for", false, "for $1 in $2 {}"),
        snippet("var", true, "let ${1:$NAME} = $EXPR;$0"),
    ];
    let text = "    get_user(id).v";
    let found = from_snippets(text, text.len(), "rust", &snippets, "    ");
    assert_eq!(labels(&found), ["var"]);
    assert_eq!(found[0].replace, 4..text.len());
    assert_eq!(found[0].new_text, "let user = get_user(id);");
    assert_eq!(found[0].stops, [4..8, 24..24]);
}

#[test]
fn jig_snippets_take_the_place_of_server_ones_with_their_name() {
    let ours = from_snippets("for", 3, "rust", &[snippet("for", false, "for $1 {}")], "");
    let keyword = Completion {
        item: CompletionItem {
            label: "for".into(),
            kind: Some(CompletionItemKind::KEYWORD),
            ..Default::default()
        },
        replace: 0..3,
        new_text: "for".into(),
        stops: Vec::new(),
    };
    let format = Completion {
        item: CompletionItem {
            label: "format".into(),
            ..Default::default()
        },
        ..keyword.clone()
    };
    let merged = merge("for", 3, ours, vec![keyword, format]);
    assert_eq!(labels(&merged), ["for", "format"]);
    assert_eq!(merged[0].item.kind, Some(CompletionItemKind::SNIPPET));
}

#[test]
fn requests_say_whether_a_trigger_character_was_typed() {
    let triggers = [".".to_string(), ":".to_string()];
    assert_eq!(
        request_context("v.", 2, &triggers),
        json!({"triggerKind": 2, "triggerCharacter": "."})
    );
    assert_eq!(
        request_context("v.pu", 4, &triggers),
        json!({"triggerKind": 1})
    );
}

#[test]
fn the_highlight_never_ends_inside_a_character() {
    assert_eq!(highlighted("push(…)", "pu"), "pu");
    assert_eq!(highlighted("push(…)", "push(x"), "push(");
    assert_eq!(highlighted("len", "length"), "len");

    let text = "fn f(v: Vec<u8>) { v.pu }";
    let offset = text.find(" }").unwrap();
    let rest = vec![Completion {
        item: CompletionItem {
            label: "push(…)".into(),
            ..Default::default()
        },
        replace: offset - 2..offset,
        new_text: "push()".into(),
        stops: Vec::new(),
    }];
    let found = merge(text, offset, Vec::new(), rest);
    assert_eq!(found[0].item.filter_text.as_deref(), Some("pu"));
}
