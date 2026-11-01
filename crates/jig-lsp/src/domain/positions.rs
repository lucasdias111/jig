//! Positions as the protocol counts them, and byte offsets in the text.

use std::ops::Range;

/// How a server counts the columns in a position.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Encoding {
    Utf8,
    /// The protocol's default.
    Utf16,
}

/// The LSP position of byte `offset` in `text`.
pub fn position(text: &str, offset: usize, encoding: Encoding) -> lsp_types::Position {
    let offset = offset.min(text.len());
    let line_start = text[..offset].rfind('\n').map_or(0, |ix| ix + 1);
    let line = text[..line_start].matches('\n').count();
    let column = &text[line_start..offset];
    let character = match encoding {
        Encoding::Utf8 => column.len(),
        Encoding::Utf16 => column.encode_utf16().count(),
    };
    lsp_types::Position::new(line as u32, character as u32)
}

/// The byte offset of an LSP position in `text`, clamped to its line.
pub fn offset(text: &str, position: lsp_types::Position, encoding: Encoding) -> usize {
    let mut line_start = 0;
    for _ in 0..position.line {
        match text[line_start..].find('\n') {
            Some(ix) => line_start += ix + 1,
            None => return text.len(),
        }
    }
    let line_end = text[line_start..]
        .find('\n')
        .map_or(text.len(), |ix| line_start + ix);
    let line = &text[line_start..line_end];
    let wanted = position.character as usize;
    let mut units = 0;
    for (ix, c) in line.char_indices() {
        if units >= wanted {
            return line_start + ix;
        }
        units += match encoding {
            Encoding::Utf8 => c.len_utf8(),
            Encoding::Utf16 => c.len_utf16(),
        };
    }
    line_end
}

pub fn range(text: &str, range: lsp_types::Range, encoding: Encoding) -> Range<usize> {
    offset(text, range.start, encoding)..offset(text, range.end, encoding)
}
