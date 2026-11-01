//! The two tidy-ups made on save: trailing whitespace and the final
//! newline, with the line endings `.editorconfig` asks for.

use jig_kit::editorconfig::LineEnding;

/// `text` with trailing spaces and tabs taken off each line, its line
/// endings made `end_of_line`, and ending in a newline if it has any text,
/// as asked.
pub fn tidy(
    text: &str,
    trim_trailing_whitespace: bool,
    final_newline: bool,
    end_of_line: Option<LineEnding>,
) -> String {
    let mut out = if trim_trailing_whitespace {
        let mut out = String::with_capacity(text.len());
        for line in text.split_inclusive('\n') {
            let body = line.trim_end_matches(['\n', '\r']);
            out.push_str(body.trim_end_matches([' ', '\t']));
            out.push_str(&line[body.len()..]);
        }
        out
    } else {
        text.to_string()
    };
    if let Some(ending) = end_of_line {
        out = out.replace("\r\n", "\n");
        if ending == LineEnding::CrLf {
            out = out.replace('\n', "\r\n");
        }
    }
    if final_newline && !out.is_empty() && !out.ends_with('\n') {
        let ending = end_of_line.unwrap_or(if text.contains("\r\n") {
            LineEnding::CrLf
        } else {
            LineEnding::Lf
        });
        out.push_str(ending.as_str());
    }
    out
}
