//! What to suggest while typing a name: the language server's completions,
//! narrowed to what's been typed, or without a server, other words in the
//! file; and Jig's [`crate::domain::snippets`] for the language.
//!
//! Positions here are byte offsets; the editor's own are worked out by the
//! caller.

use std::collections::HashMap;
use std::ops::Range;

use lsp_types::{
    CompletionItem, CompletionItemKind, CompletionResponse, CompletionTextEdit, InsertTextFormat,
};

use self::ranking::{SUBSEQUENCE, match_tier, words};
use crate::domain::snippets::{self, Snippet};
use jig_lsp::lsp::{self, Encoding};

/// Most suggestions shown at once.
const MAX_ITEMS: usize = 100;
/// Words are suggested from the first letter, as other editors do; the
/// nearest good matches come first, so a short prefix still lands well.
const MIN_WORD_PREFIX: usize = 1;
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

/// Whether `offset` is where a struct literal's next field could go: just
/// after its `{` or a `,`, maybe past spaces or a new line, with nothing of
/// a name typed yet. Typing there asks the server, and its fields are
/// listed if that's all it offers ([`only_fields`]).
pub fn could_start_a_field(text: &str, offset: usize) -> bool {
    let offset = offset.min(text.len());
    if name_start(text, offset) != offset {
        return false;
    }
    text[..offset].trim_end().ends_with(['{', ','])
}

/// Whether `found` is a list of fields to fill in, as a server offers in a
/// struct or object literal, rather than everything that could go in a
/// block or a call's arguments.
pub fn only_fields(found: &[Completion]) -> bool {
    !found.is_empty()
        && found.iter().all(|completion| {
            matches!(
                completion.item.kind,
                Some(CompletionItemKind::FIELD | CompletionItemKind::PROPERTY)
            ) || completion.item.label.starts_with("..")
        })
}

/// Other words in `text` that start with the one being typed at `offset`,
/// nearest first.
pub fn from_words(text: &str, offset: usize) -> Vec<Completion> {
    let offset = offset.min(text.len());
    let start = name_start(text, offset);
    let typed = &text[start..offset];
    // A number being typed isn't the start of a word.
    if typed.chars().count() < MIN_WORD_PREFIX
        || typed.starts_with(|c: char| c.is_ascii_digit())
        || text.len() > MAX_WORD_SCAN_BYTES
    {
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

mod ranking;

#[cfg(test)]
mod tests;
