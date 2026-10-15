//! Each language the blueprint view reads besides Rust, as a `Syntax` the
//! shared flow builder reads it by.

mod c;
mod go;
mod java;
mod python;
mod script;
mod swift;
#[cfg(test)]
mod testing;
mod zig;

use super::Lang;
use super::flow::Syntax;

/// How `lang` reads, or `None` for Rust, which has its own reader.
pub(super) fn syntax(lang: Lang) -> Option<&'static Syntax> {
    Some(match lang {
        Lang::Rust => return None,
        Lang::JavaScript | Lang::TypeScript | Lang::Tsx => &script::SYNTAX,
        Lang::Python => &python::SYNTAX,
        Lang::Go => &go::SYNTAX,
        Lang::Java => &java::SYNTAX,
        Lang::C => &c::C,
        Lang::Cpp => &c::CPP,
        Lang::Swift => &swift::SYNTAX,
        Lang::Zig => &zig::SYNTAX,
    })
}
