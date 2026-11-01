use super::*;

/// The text of each place in `snippet`, in order.
fn places(snippet: &str) -> Vec<String> {
    let expansion = parse(snippet);
    expansion
        .stops
        .iter()
        .map(|r| expansion.text[r.clone()].to_string())
        .collect()
}

#[test]
fn places_in_order_then_the_end() {
    let expansion = parse("for ${1:item} in ${2:items} {\n\t$0\n}");
    assert_eq!(expansion.text, "for item in items {\n\t\n}");
    assert_eq!(expansion.stops, [4..8, 12..17, 21..21]);
}

#[test]
fn without_a_final_place_the_end_is_last() {
    let expansion = parse("push(${1:value})");
    assert_eq!(expansion.stops, [5..10, 11..11]);
}

#[test]
fn plain_text_has_no_places() {
    assert!(parse("push").stops.is_empty());
    assert!(parse("push()$0").stops.is_empty());
    assert_eq!(parse("println!(\"$0\")").stops.first(), Some(&(10..10)));
}

#[test]
fn nested_choices_variables_and_escapes() {
    assert_eq!(parse("${1:Vec<${2:T}>}").text, "Vec<T>");
    assert_eq!(places("${1:Vec<${2:T}>}"), ["Vec<T>", "T", ""]);
    assert_eq!(parse("${1|one,two|}").text, "one");
    assert_eq!(parse("$TM_SELECTED_TEXT${TM_FILENAME:x}").text, "x");
    assert_eq!(parse("cost \\$5 \\} $").text, "cost $5 } $");
}

#[test]
fn a_place_repeated_repeats_its_text() {
    let snippet = "for (let ${1:i} = 0; $1 < n; $1++) {$0}";
    assert_eq!(parse(snippet).text, "for (let i = 0; i < n; i++) {}");
    assert_eq!(places(snippet), ["i", ""]);
}

#[test]
fn indented_to_the_line() {
    assert_eq!(
        indent("if x {\n\t$0\n}", "    ", "    "),
        "if x {\n        $0\n    }"
    );
}

#[test]
fn receivers() {
    fn at(text: &str) -> Option<&str> {
        let dot = text.rfind('.').unwrap();
        receiver(text, dot).map(|r| &text[r])
    }
    assert_eq!(at("    users.v"), Some("users"));
    assert_eq!(at("let x = self.items.len()."), Some("self.items.len()"));
    assert_eq!(at("foo(bar(1), [2]).x"), Some("foo(bar(1), [2])"));
    assert_eq!(at("x = Vec::new()."), Some("Vec::new()"));
    assert_eq!(at("vec![1, 2]."), Some("vec![1, 2]"));
    assert_eq!(at("\"a.b\"."), Some("\"a.b\""));
    assert_eq!(at("a?.b?.c."), Some("a?.b?.c"));
    assert_eq!(at("load()?."), Some("load()?"));
    assert_eq!(at("  ."), None);
}

#[test]
fn names() {
    assert_eq!(name_for("users", "rust"), "users");
    assert_eq!(name_for("get_user(id)", "rust"), "user");
    assert_eq!(name_for("getUser(id)", "typescript"), "user");
    assert_eq!(name_for("HttpClient::new()", "rust"), "http_client");
    assert_eq!(name_for("HttpClient::new()", "go"), "httpClient");
    assert_eq!(name_for("self.items.clone()", "rust"), "items");
    assert_eq!(name_for("\"hello\"", "python"), "value");
    assert_eq!(name_for("fetchURLList()", "javascript"), "urlList");
    assert_eq!(name_for("42", "rust"), "value");
}

#[test]
fn postfix_bodies_get_the_expression_and_a_name() {
    assert_eq!(
        parse(&wrap(
            "let ${1:$NAME} = $EXPR;",
            "get_user(\"$x}\")",
            "rust"
        ))
        .text,
        "let user = get_user(\"$x}\");"
    );
}
