//! How the Settings window puts lists into words.

/// `"a"`, `"a and b"`, `"a, b and c"`.
pub fn sentence(labels: &[&str]) -> String {
    match labels {
        [] => String::new(),
        [one] => one.to_string(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn joins_with_commas_then_and() {
        assert_eq!(sentence(&[]), "");
        assert_eq!(sentence(&["Rust"]), "Rust");
        assert_eq!(sentence(&["TS", "TSX", "JS"]), "TS, TSX and JS");
    }
}
