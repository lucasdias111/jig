//! Reading a shell command line as the shell would: its simple commands
//! and their words, or nothing when the shell would put something in.

/// A simple command's words, quotes taken off. `None` when the shell would
/// put something in them: a variable, or `~` for the home folder.
pub(super) fn words(part: &str) -> Option<Vec<String>> {
    let chars: Vec<char> = part.chars().collect();
    let mut words = Vec::new();
    let mut word: Option<String> = None;
    let mut quote: Option<char> = None;
    let mut at = 0;
    while at < chars.len() {
        let c = chars[at];
        let next = chars.get(at + 1).copied();
        let expands = c == '$'
            && quote != Some('\'')
            && next.is_some_and(|n| n.is_alphanumeric() || matches!(n, '_' | '{' | '@' | '*'));
        if expands {
            return None;
        }
        match (quote, c) {
            (Some(open), _) if c == open => quote = None,
            (Some('"'), '\\') if next.is_some_and(|n| matches!(n, '"' | '\\' | '$' | '`')) => {
                word.get_or_insert_default().push(next.unwrap_or_default());
                at += 1;
            }
            (Some(_), _) => word.get_or_insert_default().push(c),
            (None, '\'' | '"') => {
                quote = Some(c);
                word.get_or_insert_default();
            }
            (None, '\\') => {
                if let Some(next) = next {
                    word.get_or_insert_default().push(next);
                    at += 1;
                }
            }
            (None, '~') if word.is_none() => return None,
            (None, c) if c.is_whitespace() => words.extend(word.take()),
            (None, c) => word.get_or_insert_default().push(c),
        }
        at += 1;
    }
    words.extend(word);
    Some(words)
}

/// A command's simple commands, split at `|`, `;`, `&` and new lines
/// outside quotes, with redirections that throw output away
/// (`2>/dev/null`, `>/dev/null`, `&>/dev/null`, `2>&1`) left out. `None`
/// when it writes anywhere else or runs a command inside it.
pub(super) fn simple_commands(command: &str) -> Option<Vec<String>> {
    let chars: Vec<char> = command.chars().collect();
    let mut parts = vec![String::new()];
    let mut quote: Option<char> = None;
    let mut at = 0;
    while at < chars.len() {
        let c = chars[at];
        let next = chars.get(at + 1).copied();
        let part = parts.last_mut().expect("one part at least");
        match quote {
            Some(open) => {
                // Double quotes still substitute commands.
                if open == '"' && (c == '`' || (c == '$' && next == Some('('))) {
                    return None;
                }
                if c == open {
                    quote = None;
                } else if c == '\\'
                    && open == '"'
                    && let Some(next) = next
                {
                    part.push(c);
                    part.push(next);
                    at += 2;
                    continue;
                }
                part.push(c);
            }
            None => match c {
                '\'' | '"' => {
                    quote = Some(c);
                    part.push(c);
                }
                '`' => return None,
                '$' | '<' if next == Some('(') => return None,
                '\\' => {
                    part.push(c);
                    if let Some(next) = next {
                        part.push(next);
                        at += 1;
                    }
                }
                '&' if next == Some('>') => {}
                '|' | ';' | '&' | '\n' => parts.push(String::new()),
                '>' => {
                    // The descriptor written against it goes too.
                    while part.ends_with(|c: char| c.is_ascii_digit()) {
                        part.pop();
                    }
                    let rest: String = chars[at + 1..].iter().collect();
                    if rest.starts_with("&1") || rest.starts_with("&2") {
                        at += 3;
                        continue;
                    }
                    let target = rest.trim_start();
                    let spaces = rest.chars().count() - target.chars().count();
                    let quiet = target.strip_prefix("/dev/null").is_some_and(|after| {
                        after
                            .chars()
                            .next()
                            .is_none_or(|c| c.is_whitespace() || matches!(c, ';' | '|' | '&' | ')'))
                    });
                    if !quiet {
                        return None;
                    }
                    part.push(' ');
                    at += 1 + spaces + "/dev/null".len();
                    continue;
                }
                _ => part.push(c),
            },
        }
        at += 1;
    }
    Some(
        parts
            .into_iter()
            .map(|part| part.trim().to_string())
            .filter(|part| !part.is_empty())
            .collect(),
    )
}
