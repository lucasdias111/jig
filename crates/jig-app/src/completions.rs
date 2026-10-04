//! What to suggest while typing a name: the language server's completions,
//! narrowed to what's been typed, or without a server, other words in the
//! file; and Jig's [`crate::snippets`] for the language.
//!
//! Positions here are byte offsets; the editor's own are worked out by the
//! caller.

use std::collections::HashMap;
use std::ops::Range;

use lsp_types::{
    CompletionItem, CompletionItemKind, CompletionResponse, CompletionTextEdit, InsertTextFormat,
};

use crate::lsp::{self, Encoding};
use crate::snippets::{self, Snippet};

/// Most suggestions shown at once.
const MAX_ITEMS: usize = 100;
/// Words are only suggested once this much of one has been typed, so the
/// list doesn't pop up for every short name.
const MIN_WORD_PREFIX: usize = 3;
/// Likewise snippets, unless their whole name has been typed.
const MIN_SNIPPET_PREFIX: usize = 2;
const MAX_WORDS: usize = 20;
/// Files larger than this aren't scanned for words on each keystroke.
const MAX_WORD_SCAN_BYTES: usize = 1024 * 1024;

/// One suggestion: what the list shows, and the text that replaces `replace`
/// when it's taken.
#[derive(Clone, Debug, PartialEq)]
pub struct Completion {
    pub item: CompletionItem,
    pub replace: Range<usize>,
    pub new_text: String,
    /// For a snippet, the places in `new_text` to fill in, in order.
    pub stops: Vec<Range<usize>>,
}

pub fn is_name_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '$'
}

/// Where the name being typed at `offset` starts.
pub fn name_start(text: &str, offset: usize) -> usize {
    let offset = offset.min(text.len());
    text[..offset]
        .char_indices()
        .rev()
        .take_while(|(_, c)| is_name_char(*c))
        .last()
        .map_or(offset, |(ix, _)| ix)
}

/// The `context` of a completion request at `offset`: typed after one of
/// the server's trigger characters, such as `.`, or invoked while typing a
/// name.
pub fn request_context(text: &str, offset: usize, triggers: &[String]) -> serde_json::Value {
    let offset = offset.min(text.len());
    let trigger = (name_start(text, offset) == offset)
        .then(|| {
            triggers
                .iter()
                .find(|t| text[..offset].ends_with(t.as_str()))
        })
        .flatten();
    match trigger {
        Some(trigger) => serde_json::json!({"triggerKind": 2, "triggerCharacter": trigger}),
        None => serde_json::json!({"triggerKind": 1}),
    }
}

/// The server's answer to a completion request made at `offset` in `text`,
/// narrowed to what's been typed and best first.
pub fn from_server(
    text: &str,
    offset: usize,
    encoding: Encoding,
    response: &serde_json::Value,
    unit: &str,
) -> Vec<Completion> {
    let items = match serde_json::from_value::<Option<CompletionResponse>>(response.clone()) {
        Ok(Some(CompletionResponse::Array(items))) => items,
        Ok(Some(CompletionResponse::List(list))) => list.items,
        _ => return Vec::new(),
    };
    let offset = offset.min(text.len());
    let typed_from = name_start(text, offset);
    let mut found: Vec<(u8, Completion)> = items
        .into_iter()
        .filter_map(|item| {
            let (replace, new_text) = match &item.text_edit {
                Some(CompletionTextEdit::Edit(edit)) => (
                    lsp::range(text, edit.range, encoding),
                    edit.new_text.clone(),
                ),
                // Insert, as typing would, rather than eat the rest of a word.
                Some(CompletionTextEdit::InsertAndReplace(edit)) => (
                    lsp::range(text, edit.insert, encoding),
                    edit.new_text.clone(),
                ),
                None => (
                    typed_from..offset,
                    item.insert_text
                        .clone()
                        .unwrap_or_else(|| item.label.clone()),
                ),
            };
            let (mut new_text, mut stops) =
                if item.insert_text_format == Some(InsertTextFormat::SNIPPET) {
                    let expansion = snippets::parse(&snippets::indent(
                        &new_text,
                        line_indent(text, offset),
                        unit,
                    ));
                    (expansion.text, expansion.stops)
                } else {
                    (new_text, Vec::new())
                };
            let filter = item.filter_text.as_deref().unwrap_or(&item.label);
            // Postfix items replace the expression too, but are named after
            // only what follows its dot.
            let tier = match_tier(filter, text.get(replace.start.min(offset)..offset)?)
                .or_else(|| match_tier(filter, &text[typed_from..offset]))?;
            // Postfix items from rust-analyzer remove their expression with
            // an extra edit just before their own; the editor makes only one,
            // so join them. Items with edits elsewhere, such as imports, would
            // be left half done, so aren't offered.
            let mut replace = replace;
            for extra in item.additional_text_edits.iter().flatten() {
                let range = lsp::range(text, extra.range, encoding);
                if range.end != replace.start {
                    return None;
                }
                replace.start = range.start;
                new_text.insert_str(0, &extra.new_text);
                for stop in &mut stops {
                    *stop = stop.start + extra.new_text.len()..stop.end + extra.new_text.len();
                }
            }
            let item = CompletionItem {
                // Set by `merge` to what the menu should highlight.
                filter_text: None,
                text_edit: None,
                insert_text: None,
                additional_text_edits: None,
                ..item
            };
            Some((
                tier,
                Completion {
                    item,
                    replace,
                    new_text,
                    stops,
                },
            ))
        })
        .collect();
    // Stable: the server's own order breaks ties.
    found.sort_by(|(a, x), (b, y)| {
        let key = |c: &Completion| {
            c.item
                .sort_text
                .clone()
                .unwrap_or_else(|| c.item.label.clone())
        };
        a.cmp(b).then_with(|| key(x).cmp(&key(y)))
    });
    found.truncate(MAX_ITEMS);
    found
        .into_iter()
        .map(|(_, completion)| completion)
        .collect()
}

/// Other words in `text` that start with the one being typed at `offset`,
/// nearest first.
pub fn from_words(text: &str, offset: usize) -> Vec<Completion> {
    let offset = offset.min(text.len());
    let start = name_start(text, offset);
    let typed = &text[start..offset];
    if typed.chars().count() < MIN_WORD_PREFIX || text.len() > MAX_WORD_SCAN_BYTES {
        return Vec::new();
    }
    // Each word, the tier it matches in, and how far its nearest use is.
    let mut nearest: HashMap<&str, (u8, usize)> = HashMap::new();
    for (word_start, word) in words(text) {
        if word_start == start || word.len() <= typed.len() {
            continue;
        }
        let Some(tier) = match_tier(word, typed).filter(|tier| *tier < SUBSEQUENCE) else {
            continue;
        };
        let distance = word_start.abs_diff(offset);
        nearest
            .entry(word)
            .and_modify(|(_, best)| *best = (*best).min(distance))
            .or_insert((tier, distance));
    }
    let mut found: Vec<(&str, (u8, usize))> = nearest.into_iter().collect();
    found.sort_by_key(|(word, rank)| (*rank, *word));
    found
        .into_iter()
        .take(MAX_WORDS)
        .map(|(word, _)| Completion {
            item: CompletionItem {
                label: word.to_string(),
                kind: Some(lsp_types::CompletionItemKind::TEXT),
                ..Default::default()
            },
            replace: start..offset,
            new_text: word.to_string(),
            stops: Vec::new(),
        })
        .collect()
}

/// Jig's `snippets` matching what's typed at `offset`: prefix ones by name,
/// and after an expression and a dot, postfix ones wrapping it.
pub fn from_snippets(
    text: &str,
    offset: usize,
    language: &str,
    snippets: &[Snippet],
    unit: &str,
) -> Vec<Completion> {
    let offset = offset.min(text.len());
    let start = name_start(text, offset);
    let typed = &text[start..offset];
    let receiver = start
        .checked_sub(1)
        .and_then(|dot| snippets::receiver(text, dot));
    let indent = line_indent(text, offset);
    let mut found: Vec<(u8, Completion)> = snippets
        .iter()
        .filter(|snippet| snippet.postfix == receiver.is_some())
        .filter(|snippet| {
            snippet.name == typed
                || receiver.is_some()
                || typed.chars().count() >= MIN_SNIPPET_PREFIX
        })
        .filter_map(|snippet| {
            let tier = match_tier(&snippet.name, typed).filter(|tier| *tier < SUBSEQUENCE)?;
            let body = snippets::indent(&snippet.body, indent, unit);
            let (body, replace) = match &receiver {
                Some(expr) => (
                    snippets::wrap(&body, &text[expr.clone()], language),
                    expr.start..offset,
                ),
                None => (body, start..offset),
            };
            let expansion = snippets::parse(&body);
            Some((
                tier,
                Completion {
                    item: CompletionItem {
                        label: snippet.name.clone(),
                        kind: Some(CompletionItemKind::SNIPPET),
                        detail: Some(snippet.description.clone()).filter(|d| !d.is_empty()),
                        ..Default::default()
                    },
                    replace,
                    new_text: expansion.text,
                    stops: expansion.stops,
                },
            ))
        })
        .collect();
    found.sort_by(|(a, x), (b, y)| a.cmp(b).then_with(|| x.item.label.cmp(&y.item.label)));
    found.into_iter().map(|(_, c)| c).collect()
}

/// Jig's snippets and the rest in one list: a snippet whose whole name was
/// typed first, then the rest, then other snippets. A server's keyword or
/// snippet with a Jig snippet's name gives way to it.
pub fn merge(
    text: &str,
    offset: usize,
    snippets: Vec<Completion>,
    rest: Vec<Completion>,
) -> Vec<Completion> {
    let typed = &text[name_start(text, offset.min(text.len()))..offset.min(text.len())];
    let (exact, other): (Vec<_>, Vec<_>) =
        snippets.into_iter().partition(|s| s.item.label == typed);
    let shadowed = |c: &Completion| {
        matches!(
            c.item.kind,
            Some(CompletionItemKind::SNIPPET | CompletionItemKind::KEYWORD)
        ) && exact
            .iter()
            .chain(&other)
            .any(|s| s.item.label == c.item.label)
    };
    let rest: Vec<Completion> = rest.into_iter().filter(|c| !shadowed(c)).collect();
    let mut all: Vec<Completion> = exact.into_iter().chain(rest).chain(other).collect();
    all.truncate(MAX_ITEMS);
    for completion in &mut all {
        completion.item.filter_text = Some(highlighted(&completion.item.label, typed).into());
    }
    all
}

/// How much of `label` the menu highlights: as much as was typed, ending on
/// a whole character. Without it the menu highlights from where the
/// completion starts, which can be before a dot and end inside a label's
/// `…`, and drawing that aborts.
fn highlighted<'a>(label: &'a str, typed: &str) -> &'a str {
    let mut end = typed.len().min(label.len());
    while !label.is_char_boundary(end) {
        end -= 1;
    }
    &label[..end]
}

/// The indentation of the line `offset` is on.
fn line_indent(text: &str, offset: usize) -> &str {
    let line_start = text[..offset.min(text.len())]
        .rfind('\n')
        .map_or(0, |ix| ix + 1);
    let line = &text[line_start..];
    let end = line
        .find(|c: char| c != ' ' && c != '\t')
        .unwrap_or(line.len());
    &line[..end]
}

/// The names in `text`, with where each starts.
fn words(text: &str) -> impl Iterator<Item = (usize, &str)> {
    let mut chars = text.char_indices().peekable();
    std::iter::from_fn(move || {
        loop {
            let (start, first) = chars.next()?;
            if !is_name_char(first) {
                continue;
            }
            let mut end = start + first.len_utf8();
            while let Some(&(ix, c)) = chars.peek() {
                if !is_name_char(c) {
                    break;
                }
                end = ix + c.len_utf8();
                chars.next();
            }
            if !first.is_ascii_digit() {
                return Some((start, &text[start..end]));
            }
        }
    })
}

const EXACT_PREFIX: u8 = 0;
const PREFIX: u8 = 1;
const SUBSEQUENCE: u8 = 2;

/// How well `candidate` matches what was `typed`, lower being better, or
/// `None` if it doesn't.
fn match_tier(candidate: &str, typed: &str) -> Option<u8> {
    if candidate.starts_with(typed) {
        return Some(EXACT_PREFIX);
    }
    let fold = |s: &str| s.to_lowercase();
    let (candidate, typed) = (fold(candidate), fold(typed));
    if candidate.starts_with(&typed) {
        return Some(PREFIX);
    }
    let mut rest = candidate.chars();
    typed
        .chars()
        .all(|c| rest.any(|other| other == c))
        .then_some(SUBSEQUENCE)
}

#[cfg(test)]
mod tests {
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
    fn words_wait_for_three_characters() {
        let text = "counter\nco";
        assert!(from_words(text, text.len()).is_empty());
    }

    #[test]
    fn words_match_any_case() {
        let text = "HttpClient\nhttp";
        assert_eq!(labels(&from_words(text, text.len())), ["HttpClient"]);
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
}
