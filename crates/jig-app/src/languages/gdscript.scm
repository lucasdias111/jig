; Highlights for GDScript (tree-sitter-gdscript 6.x), which ships none.
; Earlier patterns win, so the specific ones come before `identifier`.

(comment) @comment

(string) @string
(node_path) @string.special
(string_name) @string.special.symbol
(get_node) @string.special
(escape_sequence) @string.escape

(integer) @number
(float) @number
[(true) (false)] @boolean
(null) @constant

; Declarations
(function_definition name: (name) @function)
(lambda name: (name) @function)
(constructor_definition "_init" @function)
(signal_statement name: (name) @function)
(class_definition name: (name) @type)
(class_name_statement name: (name) @type)
(enum_definition name: (name) @type)
(enumerator left: (identifier) @constant)
(const_statement name: (name) @constant)
(variable_statement name: (name) @variable)
(export_variable_statement name: (name) @variable)
(onready_variable_statement name: (name) @variable)

(typed_parameter (identifier) @variable.special)
(typed_default_parameter (identifier) @variable.special)
(default_parameter (identifier) @variable.special)
(parameters (identifier) @variable.special)

; Types
(type) @type
(extends_statement (string) @string)

; Calls
(call (identifier) @function)
(attribute_call (identifier) @function)
(base_call (identifier) @function)

(attribute (identifier) @property)

(annotation "@" @attribute (identifier) @attribute)

((identifier) @variable.special
  (#eq? @variable.special "self"))
((identifier) @constant
  (#match? @constant "^[A-Z][A-Z0-9_]+$"))
((identifier) @type
  (#match? @type "^[A-Z]"))

[
  "if" "elif" "else" "for" "while" "match" "when" "break" "continue" "pass"
  "return" "await" "in" "is" "as" "and" "or" "not"
  "func" "class" "class_name" "extends" "var" "const" "enum" "signal"
  "export" "onready" "setget" "set" "get"
  "remote" "master" "puppet" "remotesync" "mastersync" "puppetsync"
] @keyword

(static_keyword) @keyword
(breakpoint_statement) @keyword
(pass_statement) @keyword
(break_statement) @keyword
(continue_statement) @keyword
(region_start) @comment
(region_end) @comment

[
  "+" "-" "*" "/" "%" "**" "=" "+=" "-=" "*=" "/=" "%=" "**=" ":="
  "==" "!=" "<" "<=" ">" ">=" "<<" ">>" "<<=" ">>=" "&" "|" "^" "~"
  "&=" "|=" "^=" "&&" "||" "!" "->"
] @operator

["(" ")" "[" "]" "{" "}"] @punctuation.bracket
["," "." ":" ";"] @punctuation.delimiter
