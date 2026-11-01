//! What a file or function is for, from its doc comment, and where its body starts.

use super::*;

/// Where each line starts, to tell which line a byte is on.
pub(super) struct Lines(Vec<usize>);

impl Lines {
    pub(super) fn of(source: &str) -> Lines {
        let mut starts = vec![0];
        starts.extend(source.match_indices('\n').map(|(at, _)| at + 1));
        Lines(starts)
    }

    /// The line (0-based) byte `at` is on.
    pub(super) fn at(&self, at: usize) -> u32 {
        (self.0.partition_point(|&start| start <= at) - 1) as u32
    }
}

/// The first statement in `body`, or the body itself when it's an
/// expression or empty.
pub(super) fn first_statement(body: SyntaxNode) -> SyntaxNode {
    let mut cursor = body.walk();
    let first = body
        .named_children(&mut cursor)
        .find(|child| !child.is_extra() && !child.kind().contains("comment"));
    match first {
        // A block's statements are the ones that run.
        Some(first) if body.kind().contains("block") || body.kind().ends_with("body") => first,
        _ => body,
    }
}

/// Comments, as tree-sitter's grammars name them.
pub(super) fn is_comment(node: SyntaxNode) -> bool {
    node.kind().contains("comment")
}

/// What the doc comment right before `node` says it's for, in a line.
pub(super) fn doc_before(node: SyntaxNode, source: &str) -> Option<String> {
    let mut comments = Vec::new();
    let mut at = node.prev_sibling();
    // Attributes and decorators sit between a type and its doc.
    while let Some(previous) = at {
        if is_comment(previous) {
            comments.push(text(previous, source));
        } else if !matches!(
            previous.kind(),
            "attribute_item" | "decorator" | "annotation"
        ) {
            break;
        }
        at = previous.prev_sibling();
    }
    comments.reverse();
    purpose_of(&comments.join("\n"))
}

/// What the file is for: its leading doc comment (`//!`, a docstring, a
/// comment before Go's `package`), unless it's a licence.
pub(super) fn file_purpose(root: SyntaxNode, source: &str, lang: Lang) -> Option<String> {
    let mut cursor = root.walk();
    let children: Vec<SyntaxNode> = root.named_children(&mut cursor).collect();
    if lang == Lang::Python
        && let Some(first) = children.iter().find(|child| !is_comment(**child))
        && first.kind() == "expression_statement"
        && first
            .named_child(0)
            .is_some_and(|node| node.kind() == "string")
    {
        return purpose_of(text(*first, source));
    }
    let leading: Vec<&str> = children
        .iter()
        .take_while(|child| is_comment(**child))
        .map(|child| text(*child, source))
        // Rust's file doc is `//!`; its `///` belongs to what follows.
        .filter(|comment| lang != Lang::Rust || comment.starts_with("//!"))
        .collect();
    purpose_of(&leading.join("\n"))
}

/// A comment's first sentence, its markers taken off, on one line; none
/// for an empty one or a licence.
pub(super) fn purpose_of(comment: &str) -> Option<String> {
    let mut words = Vec::new();
    for line in comment.lines() {
        let line = line
            .trim()
            .trim_start_matches("/**")
            .trim_start_matches("/*")
            .trim_end_matches("*/")
            .trim_start_matches(['/', '!', '*', '#'])
            .trim_matches(['"', '\''])
            .trim();
        if line.is_empty() {
            if !words.is_empty() {
                break;
            }
            continue;
        }
        if line.starts_with('@') {
            break;
        }
        words.extend(line.split_whitespace());
    }
    let text = words.join(" ");
    let lower = text.to_lowercase();
    if text.is_empty()
        || ["copyright", "license", "licence", "spdx"]
            .iter()
            .any(|word| lower.contains(word))
    {
        return None;
    }
    // Its first sentence, unless that's all it says.
    let end = text
        .match_indices(". ")
        .map(|(at, _)| at + 1)
        .find(|&at| at > 12)
        .unwrap_or(text.len());
    Some(text[..end].trim_end_matches(':').to_string())
}
