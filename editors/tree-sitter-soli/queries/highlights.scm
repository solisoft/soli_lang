; Soli highlights. Later patterns win in Neovim and Zed; Helix takes the
; first match — so specific captures come before the generic `identifier`.

; ------------------------------------------------------------------ comments

(comment) @comment
(block_comment) @comment

; ------------------------------------------------------------------ keywords

[
  "class"
  "module"
  "interface"
  "enum"
  "extends"
  "implements"
  "new"
] @keyword.type

[
  "def"
  "fn"
] @keyword.function

[
  "let"
  "const"
  "static"
] @keyword.modifier

(visibility) @keyword.modifier

[
  "return"
  "next"
  "break"
] @keyword.return

[
  "if"
  "elsif"
  "else"
  "unless"
  "then"
  "match"
] @keyword.conditional

[
  "while"
  "until"
  "for"
  "in"
  "do"
] @keyword.repeat

[
  "try"
  "catch"
  "finally"
  "throw"
  "begin"
  "rescue"
  "ensure"
] @keyword.exception

[
  "import"
  "export"
  "from"
  "as"
] @keyword.import

"end" @keyword

[
  "and"
  "or"
  "not"
] @keyword.operator

; ----------------------------------------------------------------- operators

[
  "="
  "=="
  "==="
  "!="
  "=~"
  "<"
  "<="
  ">"
  ">="
  "<=>"
  "+"
  "-"
  "*"
  "**"
  "/"
  "%"
  "!"
  "~"
  "&&"
  "||"
  "??"
  "|>"
  "<<"
  ">>"
  "+="
  "-="
  "*="
  "/="
  "%="
  "||="
  "&&="
  "??="
  "<<="
  "++"
  "--"
  ".."
  "..="
  "..."
  "->"
  "=>"
  "?"
  "&"
] @operator

[
  "."
  "&."
  "::"
  ","
  ":"
] @punctuation.delimiter

[
  "("
  ")"
  "["
  "]"
  "{"
  "}"
] @punctuation.bracket

(block_parameters "|" @punctuation.bracket)
(lambda_parameters "|" @punctuation.bracket)

(interpolation
  "#{" @punctuation.special
  "}" @punctuation.special)

; ------------------------------------------------------------------ literals

(string) @string
(raw_string) @string
(raw_string_triple) @string
(command_string) @string.special
(word_array) @string.special
(sdbql_block) @string.special

(simple_symbol) @string.special.symbol
(hash_key_symbol) @string.special.symbol
(pair key: (string) @string.special.symbol)

(integer) @number
(float) @number.float

[
  (true)
  (false)
] @boolean

(nil) @constant.builtin

; --------------------------------------------------------------- identifiers

(identifier) @variable

(instance_variable) @variable.member

(self) @variable.builtin
(super) @variable.builtin

(parameter name: (identifier) @variable.parameter)
(destructuring_parameter (identifier) @variable.parameter)

(keyword_argument name: (identifier) @variable.parameter)

(property_identifier) @variable.member

(class_field name: (_) @variable.member)

((identifier) @constant
  (#match? @constant "^[A-Z][A-Z0-9_]*$"))

(constant) @type

((constant) @constant
  (#match? @constant "^[A-Z][A-Z0-9_]+$"))

(constant_declaration name: (_) @constant)

(enum_variant name: (constant) @constant)

(type_identifier) @type
(scope_resolution name: (constant) @type)

((type_identifier) @type.builtin
  (#any-of? @type.builtin "Int" "Float" "Decimal" "String" "Bool" "Void" "Any" "Array" "Hash" "nil" "null"))

; --------------------------------------------------------------- functions

(function_declaration name: (_) @function.method)
(method_signature name: (_) @function.method)
(constructor "new" @constructor)

(call function: (identifier) @function.call)
(call function: (member_expression property: (property_identifier) @function.method.call))
(command_call function: (identifier) @function.call)

(new_expression "new" @keyword.operator)

((call function: (identifier) @function.builtin)
  (#any-of? @function.builtin
    "print" "println" "puts" "render" "redirect" "respond" "partial" "params" "permit"
    "describe" "context" "test" "it" "before_each" "after_each" "before_all" "after_all"
    "expect" "assert" "assert_eq" "assert_ne" "assert_null" "assert_not_null" "assert_raises"
    "get" "post" "put" "patch" "delete" "resources" "resource" "namespace" "scope"
    "has_many" "has_one" "belongs_to" "validates" "before_action" "after_action"
    "session_get" "session_set" "set_cookie" "read_cookie" "grouped" "transaction"))

((command_call function: (identifier) @function.builtin)
  (#any-of? @function.builtin
    "has_many" "has_one" "belongs_to" "has_and_belongs_to_many" "validates"
    "before_save" "after_save" "before_create" "after_create" "before_update" "after_update"
    "before_destroy" "after_destroy" "before_action" "after_action" "include" "extend"))

; `user.admin?`, `post.save!`: a method called without parentheses.
((property_identifier) @function.method.call
  (#match? @function.method.call "[?!]$"))
