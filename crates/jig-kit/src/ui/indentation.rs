//! The indentation the editor's input takes ([`TabSize`]), from the
//! domain's [`Indent`]: what a file uses, and what its `.editorconfig` says.

use gpui_kit::component::input::TabSize;

use crate::domain::editorconfig::EditorConfig;
use crate::domain::indentation::{self, Indent};

/// Used for new files and files with no indented lines.
pub const DEFAULT: TabSize = tab_size(indentation::DEFAULT);

/// Guess `text`'s indentation ([`indentation::detect`]).
pub fn detect(text: &str) -> TabSize {
    tab_size(indentation::detect(text))
}

/// As the status bar shows it: `"Spaces: 4"` or `"Tab Size: 4"`.
pub fn label(tab: TabSize) -> String {
    indentation::label(indent(tab))
}

impl EditorConfig {
    /// `detected` with whatever the file sets of its style and size.
    pub fn tab_size(&self, detected: TabSize) -> TabSize {
        tab_size(self.indent(indent(detected)))
    }
}

const fn tab_size(indent: Indent) -> TabSize {
    TabSize {
        tab_size: indent.width,
        hard_tabs: indent.hard_tabs,
    }
}

fn indent(tab: TabSize) -> Indent {
    Indent {
        width: tab.tab_size,
        hard_tabs: tab.hard_tabs,
    }
}
