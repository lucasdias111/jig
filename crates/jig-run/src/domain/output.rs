//! A run's output as shown: lines with their colours, and the files they
//! mention, which clicking opens.

use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use regex::Regex;

/// Longer lines are cut, so one huge line can't stall layout.
const MAX_LINE_CHARS: usize = 4000;

pub enum RunEvent {
    Line(OutputLine),
    /// A line of JSON on stdout, kept raw for the caller to read, when the
    /// process was started for its data (`cargo build --message-format`).
    Data(String),
    /// How it ended, in words, and whether it succeeded.
    Exited {
        message: String,
        success: bool,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stream {
    Stdout,
    Stderr,
    /// Jig's own lines: the command, and how it ended.
    Meta,
}

#[derive(Clone, Debug, PartialEq)]
pub struct OutputLine {
    pub text: Arc<str>,
    pub stream: Stream,
    /// Styled ranges of `text`; the rest is unstyled.
    pub spans: Vec<(Range<usize>, Style)>,
    pub links: Vec<Link>,
}

impl OutputLine {
    pub fn meta(text: impl Into<Arc<str>>) -> Self {
        Self {
            text: text.into(),
            stream: Stream::Meta,
            spans: Vec::new(),
            links: Vec::new(),
        }
    }
}

/// A file mentioned in the output, which clicking opens.
#[derive(Clone, Debug, PartialEq)]
pub struct Link {
    pub range: Range<usize>,
    pub path: PathBuf,
    /// 1-based.
    pub line: u32,
    /// 1-based.
    pub column: Option<u32>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Style {
    pub fg: Option<Color>,
    pub bold: bool,
    pub dim: bool,
    pub italic: bool,
    pub underline: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Color {
    /// One of the 16 terminal colours.
    Indexed(u8),
    Rgb(u8, u8, u8),
}

/// One line of output as shown. `style` carries colour from line to line,
/// as a terminal does; references resolve against the first of `bases`
/// they're found in.
pub fn parse_line(
    raw: &str,
    stream: Stream,
    style: &mut Style,
    bases: &[PathBuf],
    is_file: impl Fn(&Path) -> bool,
) -> OutputLine {
    let raw = raw.trim_end_matches(['\n', '\r']);
    // A carriage return redraws the line, as progress bars do: keep the last.
    let raw = raw.rsplit('\r').next().unwrap_or(raw);
    let (text, spans) = strip_ansi(raw, style);
    let links = find_links(&text, bases, is_file);
    OutputLine {
        text: text.into(),
        stream,
        spans,
        links,
    }
}

/// `raw` without escape sequences, and the ranges its colour codes styled.
pub fn strip_ansi(raw: &str, style: &mut Style) -> (String, Vec<(Range<usize>, Style)>) {
    let mut text = String::with_capacity(raw.len());
    let mut spans: Vec<(Range<usize>, Style)> = Vec::new();
    let mut chars = raw.chars().peekable();
    let mut count = 0;
    while let Some(c) = chars.next() {
        match c {
            '\x1b' => match chars.next() {
                Some('[') => {
                    let mut params = String::new();
                    for c in chars.by_ref() {
                        if ('\x40'..='\x7e').contains(&c) {
                            if c == 'm' {
                                apply_sgr(&params, style);
                            }
                            break;
                        }
                        params.push(c);
                    }
                }
                // Operating system commands, e.g. hyperlinks: up to BEL or ESC \.
                Some(']') => {
                    while let Some(c) = chars.next() {
                        if c == '\x07' || (c == '\x1b' && chars.next_if_eq(&'\\').is_some()) {
                            break;
                        }
                    }
                }
                _ => {}
            },
            '\t' => {
                let spaces = 4 - count % 4;
                push(&mut text, &mut spans, &" ".repeat(spaces), *style);
                count += spaces;
            }
            c if c.is_control() => {}
            c => {
                if count >= MAX_LINE_CHARS {
                    text.push('…');
                    break;
                }
                push(&mut text, &mut spans, c.encode_utf8(&mut [0; 4]), *style);
                count += 1;
            }
        }
    }
    (text, spans)
}

fn push(text: &mut String, spans: &mut Vec<(Range<usize>, Style)>, s: &str, style: Style) {
    let start = text.len();
    text.push_str(s);
    if style == Style::default() {
        return;
    }
    match spans.last_mut() {
        Some((range, last)) if *last == style && range.end == start => range.end = text.len(),
        _ => spans.push((start..text.len(), style)),
    }
}

/// Apply a Select Graphic Rendition sequence's parameters to `style`.
fn apply_sgr(params: &str, style: &mut Style) {
    let codes: Vec<u16> = params
        .split([';', ':'])
        .map(|p| p.parse().unwrap_or(0))
        .collect();
    let mut codes = codes.into_iter();
    while let Some(code) = codes.next() {
        match code {
            0 => *style = Style::default(),
            1 => style.bold = true,
            2 => style.dim = true,
            3 => style.italic = true,
            4 => style.underline = true,
            22 => (style.bold, style.dim) = (false, false),
            23 => style.italic = false,
            24 => style.underline = false,
            30..=37 => style.fg = Some(Color::Indexed((code - 30) as u8)),
            90..=97 => style.fg = Some(Color::Indexed((code - 90 + 8) as u8)),
            39 => style.fg = None,
            38 | 48 => {
                let color = match codes.next() {
                    Some(5) => codes.next().map(|n| match n {
                        0..=15 => Color::Indexed(n as u8),
                        n => indexed_rgb(n as u8),
                    }),
                    Some(2) => {
                        let mut channel = || codes.next().unwrap_or(0).min(255) as u8;
                        Some(Color::Rgb(channel(), channel(), channel()))
                    }
                    _ => None,
                };
                if code == 38 {
                    style.fg = color;
                }
            }
            _ => {}
        }
    }
}

/// A colour of the 256-colour palette past the first 16.
fn indexed_rgb(n: u8) -> Color {
    if n >= 232 {
        let level = 8 + (n - 232) * 10;
        return Color::Rgb(level, level, level);
    }
    let n = n - 16;
    let level = |v: u8| if v == 0 { 0 } else { 55 + v * 40 };
    Color::Rgb(level(n / 36), level(n / 6 % 6), level(n % 6))
}

fn path_pattern() -> &'static Regex {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    // `src/main.rs:12:5`, `/abs/file.ts:3`, `./a.py:9`; a file needs an
    // extension, so times like `12:30:01` don't match.
    PATTERN.get_or_init(|| {
        Regex::new(
            r"(?:^|[^\w./@+-])((?:\.{0,2}/)?(?:[\w.@+-]+/)*[\w@+-][\w.@+-]*\.\w+):(\d+)(?::(\d+))?",
        )
        .unwrap()
    })
}

fn python_pattern() -> &'static Regex {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    PATTERN.get_or_init(|| Regex::new(r#"File "([^"]+)", line (\d+)"#).unwrap())
}

/// References in `text` to files that exist.
pub fn find_links(text: &str, bases: &[PathBuf], is_file: impl Fn(&Path) -> bool) -> Vec<Link> {
    if !text.contains(':') && !text.contains("line ") {
        return Vec::new();
    }
    let mut links = Vec::new();
    for pattern in [path_pattern(), python_pattern()] {
        for captures in pattern.captures_iter(text) {
            let (Some(file), Some(line)) = (captures.get(1), captures.get(2)) else {
                continue;
            };
            let Some(path) = resolve(file.as_str(), bases, &is_file) else {
                continue;
            };
            let column = captures.get(3);
            let end = column.map_or(line.end(), |c| c.end());
            links.push(Link {
                range: file.start()..end,
                path,
                line: line.as_str().parse().unwrap_or(1).max(1),
                column: column.and_then(|c| c.as_str().parse().ok()),
            });
        }
    }
    links.sort_by_key(|link| link.range.start);
    links
}

fn resolve(file: &str, bases: &[PathBuf], is_file: impl Fn(&Path) -> bool) -> Option<PathBuf> {
    let path = Path::new(file);
    if path.is_absolute() {
        return is_file(path).then(|| path.to_path_buf());
    }
    bases
        .iter()
        .map(|base| base.join(path))
        .find(|candidate| is_file(candidate))
}
