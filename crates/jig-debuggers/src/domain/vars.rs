//! Filling `${variables}` into a debugger's commands and `launch`
//! template.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde_json::Value;

/// The variables Jig finds for itself, the tools on this computer
/// (`${node}`, `${lldb_dap}`), looked for only when used.
pub trait Tools: Sync {
    /// The value of `name`, or why there's none; `None` when `name` isn't
    /// a tool's.
    fn find(&self, name: &str, cwd: &Path) -> Option<Result<Value, String>>;
}

/// Values for `${variables}`. Tools are looked for only when used.
pub struct Vars {
    values: HashMap<String, Value>,
    cwd: PathBuf,
    tools: &'static dyn Tools,
}

impl Vars {
    /// Variables for a debugger run in `cwd`, finding tools with `tools`.
    pub fn with_tools(cwd: &Path, tools: &'static dyn Tools) -> Self {
        Self {
            values: HashMap::new(),
            cwd: cwd.to_path_buf(),
            tools,
        }
    }

    pub fn set(&mut self, name: &str, value: Value) -> &mut Self {
        self.values.insert(name.to_string(), value);
        self
    }

    pub fn get(&self, name: &str) -> Result<Value, String> {
        if let Some(value) = self.values.get(name) {
            return Ok(value.clone());
        }
        self.tools.find(name, &self.cwd).unwrap_or_else(|| {
            Err(format!(
                "debuggers.toml uses ${{{name}}}, which Jig doesn't know."
            ))
        })
    }

    /// `text` with its variables filled in, as text.
    pub fn expand_str(&self, text: &str) -> Result<String, String> {
        let mut out = String::new();
        let mut rest = text;
        while let Some(start) = rest.find("${") {
            out.push_str(&rest[..start]);
            let after = &rest[start + 2..];
            let Some(end) = after.find('}') else {
                out.push_str(&rest[start..]);
                return Ok(out);
            };
            match self.get(&after[..end])? {
                Value::String(s) => out.push_str(&s),
                Value::Null => {}
                other => out.push_str(&other.to_string()),
            }
            rest = &after[end + 1..];
        }
        out.push_str(rest);
        Ok(out)
    }

    /// `value` with its variables filled in. A string that is just one
    /// variable becomes that variable's value, list or table as it is.
    pub fn expand(&self, value: &Value) -> Result<Value, String> {
        Ok(match value {
            Value::String(text) => {
                let whole = text
                    .strip_prefix("${")
                    .and_then(|t| t.strip_suffix('}'))
                    .filter(|name| !name.contains(['$', '{', '}']));
                match whole {
                    Some(name) => self.get(name)?,
                    None => Value::String(self.expand_str(text)?),
                }
            }
            Value::Array(items) => Value::Array(
                items
                    .iter()
                    .map(|item| self.expand(item))
                    .collect::<Result<_, _>>()?,
            ),
            Value::Object(fields) => Value::Object(
                fields
                    .iter()
                    .map(|(key, value)| Ok((key.clone(), self.expand(value)?)))
                    .collect::<Result<_, String>>()?,
            ),
            other => other.clone(),
        })
    }
}
