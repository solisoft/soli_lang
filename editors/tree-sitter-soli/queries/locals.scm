[
  (function_declaration)
  (constructor)
  (lambda)
  (block)
  (do_block)
  (class_declaration)
  (module_declaration)
] @local.scope

(parameter name: (identifier) @local.definition)
(destructuring_parameter (identifier) @local.definition)
(assignment left: (identifier) @local.definition)
(variable_declaration name: (identifier) @local.definition)
(for_statement (identifier) @local.definition)

(identifier) @local.reference
