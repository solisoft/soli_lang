/**
 * @file Soli grammar for tree-sitter
 * @license MIT
 *
 * The tree is for editors — highlighting, folding, indentation, outline —
 * not for running code: where Soli's parser decides something from context
 * this grammar may accept a little more than Soli does.
 */

/// <reference types="tree-sitter-cli/dsl" />
// @ts-check

const PREC = {
  RESCUE: 1,
  ASSIGN: 2,
  LAMBDA: 3,
  TERNARY: 4,
  NULLISH: 5,
  OR: 6,
  AND: 7,
  NOT: 8,
  EQUALITY: 9,
  COMPARE: 10,
  RANGE: 11,
  PIPELINE: 12,
  SHIFT: 13,
  ADD: 14,
  MULTIPLY: 15,
  UNARY: 16,
  POSTFIX: 17,
  CALL: 18,
  MEMBER: 19,
};

const commaSep1 = (rule) => seq(rule, repeat(seq(',', rule)));
const commaSep = (rule) => optional(commaSep1(rule));
// Items of a bracketed list: commas, line breaks inside are whitespace, and
// a trailing comma is allowed.
const listOf = (rule) => optional(seq(commaSep1(rule), optional(',')));

module.exports = grammar({
  name: 'soli',

  externals: ($) => [
    $._line_break,
    $._dq_string_content,
    $._sq_string_content,
    $._interpolation_start,
    $.raw_string_triple,
    $.sdbql_block,
    $.block_comment,
    $._error_sentinel,
  ],

  extras: ($) => [/\s/, $.comment, $.block_comment],

  word: ($) => $.identifier,

  supertypes: ($) => [$._statement, $._expression, $._primary],

  conflicts: ($) => [
    [$.parenthesized_type, $.function_type],
    [$.if_expression, $._primary],
    [$.next_statement],
    [$.break_statement],
    [$.return_statement],
    [$._brace_else],
    [$._primary, $.call],
    [$.brace_body, $.hash],
    [$._primary, $.hash],
    [$.block_parameters, $.lambda_parameters],
    [$._primary, $.command_call],

  ],

  rules: {
    source_file: ($) => optional($._statements),

    // A line break, or a `;` (the scanner reads both).
    _terminator: ($) => $._line_break,

    _statements: ($) =>
      choice(
        seq(repeat1(choice(seq($._statement, $._terminator), $._terminator)), optional($._statement)),
        $._statement,
      ),

    _statement: ($) =>
      choice(
        $.expression_statement,
        $.variable_declaration,
        $.constant_declaration,
        $.function_declaration,
        $.class_declaration,
        $.module_declaration,
        $.interface_declaration,
        $.enum_declaration,
        $.import_statement,
        $.export_statement,
        $.return_statement,
        $.break_statement,
        $.next_statement,
        $.throw_statement,
        $.while_statement,
        $.for_statement,
        $.if_modifier,
        $.unless_modifier,
        $.while_modifier,
        $.command_call,
        $.visibility_section,
      ),



    expression_statement: ($) => $._expression,

    // The right side of `=` can be a parenthesis-less call: `x = greet name: "A"`.
    _value: ($) => choice($._expression, $.command_call),

    // `break if done` is `break` under a modifier, not `break` of an `if`
    // expression: both parse, the modifier reading wins.
    if_modifier: ($) => prec.dynamic(5, prec.left(-1, seq(field('body', $._statement), 'if', field('condition', $._expression)))),
    unless_modifier: ($) =>
      prec.dynamic(5, prec.left(-1, seq(field('body', $._statement), 'unless', field('condition', $._expression)))),
    while_modifier: ($) => prec.left(-1, seq(field('body', $._statement), 'while', field('condition', $._expression))),

    variable_declaration: ($) =>
      seq(
        'let',
        field('name', $.identifier),
        optional(seq(':', field('type', $._type))),
        optional(seq('=', field('value', $._expression))),
      ),

    constant_declaration: ($) =>
      seq(
        optional('static'),
        'const',
        field('name', choice($.constant, $.identifier)),
        optional(seq(':', field('type', $._type))),
        '=',
        field('value', $._expression),
      ),

    return_statement: ($) => seq('return', optional(commaSep1($._expression))),
    break_statement: ($) => seq('break', optional($._expression)),
    next_statement: ($) => seq('next', optional($._expression)),

    throw_statement: ($) => prec.right(seq('throw', $._expression)),

    // `private` / `public` / `protected` alone on a line: the members after it.
    visibility_section: ($) => prec(-1, $.visibility),
    visibility: (_) => choice('private', 'public', 'protected'),

    // ---------------------------------------------------------------- imports

    import_statement: ($) =>
      seq(
        'import',
        choice(
          field('source', $.string),
          seq(
            choice(
              seq('{', listOf(seq(choice($.identifier, $.constant), optional(seq('as', choice($.identifier, $.constant))))), '}'),
              $.identifier,
              seq('*', 'as', $.identifier),
            ),
            'from',
            field('source', $.string),
          ),
        ),
      ),

    export_statement: ($) =>
      seq(
        'export',
        choice(
          $.function_declaration,
          $.class_declaration,
          $.module_declaration,
          $.interface_declaration,
          $.enum_declaration,
          $.constant_declaration,
          $.variable_declaration,
          seq('{', listOf($.identifier), '}'),
        ),
      ),

    // --------------------------------------------------------------- functions

    function_declaration: ($) =>
      prec.right(
        seq(
          optional($.visibility),
          optional('static'),
          choice('def', 'fn'),
          optional(seq(field('receiver', choice('self', 'this', $.constant)), '.')),
          field('name', $._method_name),
          optional(field('parameters', $.parameters)),
          optional(seq('->', field('return_type', $._type))),
          optional(
            choice(
              seq('=', field('body', $._expression)),
              field('body', $.brace_body),
              seq($._terminator, optional(field('body', alias($._statements, $.body))), 'end'),
            ),
          ),
        ),
      ),

    _method_name: ($) =>
      choice(
        $.identifier,
        $.constant,
        alias(choice('new', 'class', 'end', 'in', 'not', 'match', 'delete', 'each'), $.identifier),
        alias(/[a-zA-Z_][a-zA-Z0-9_]*[?!=]/, $.identifier),
        alias(choice('==', '+', '-', '*', '/', '%', '<', '>', '<=', '>=', '[]', '[]=', '<<', '!'), $.operator),
      ),

    parameters: ($) => seq('(', listOf($.parameter), ')'),

    parameter: ($) =>
      seq(
        optional(choice('...', '*', '**', '&')),
        field('name', choice($.identifier, $.instance_variable)),
        optional(seq(':', field('type', $._type))),
        optional(seq('=', field('default', $._expression))),
      ),

    brace_body: ($) => prec.dynamic(1, seq('{', optional($._statements), '}')),

    // ----------------------------------------------------------------- classes

    class_declaration: ($) =>
      seq(
        'class',
        field('name', $._class_name),
        optional($.type_parameters),
        optional(
          choice(
            seq(choice('<', 'extends'), field('superclass', $._class_name)),
          ),
        ),
        optional(seq(choice('implements', '~'), commaSep1($._class_name))),
        field('body', $.class_body),
      ),

    module_declaration: ($) => seq('module', field('name', $._class_name), field('body', $.class_body)),

    interface_declaration: ($) =>
      seq('interface', field('name', $._class_name), optional($.type_parameters), field('body', $.interface_body)),

    interface_body: ($) =>
      choice(
        seq('{', repeat(choice($.method_signature, $._terminator, ',')), '}'),
        seq($._terminator, repeat(choice($.method_signature, $._terminator)), 'end'),
      ),

    _class_name: ($) => choice($.constant, alias($.qualified_constant, $.scope_resolution)),

    qualified_constant: ($) => seq($.constant, repeat1(seq(token.immediate('::'), $.constant))),

    class_body: ($) =>
      choice(
        seq('{', optional($._class_items), '}'),
        seq($._terminator, optional($._class_items), 'end'),
      ),

    _class_items: ($) =>
      choice(
        seq(repeat1(choice(seq($._class_item, $._terminator), $._terminator)), optional($._class_item)),
        $._class_item,
      ),

    _class_item: ($) =>
      choice(
        $.class_field,
        $.constructor,
        $.static_block,
        $._statement,
      ),

    class_field: ($) =>
      seq(
        optional($.visibility),
        optional('static'),
        field('name', choice($.identifier, $.constant)),
        token.immediate(':'),
        field('type', $._type),
        optional(seq('=', field('default', $._expression))),
      ),

    constructor: ($) =>
      seq(
        'new',
        optional(field('parameters', $.parameters)),
        choice(field('body', $.brace_body), seq($._terminator, optional(field('body', alias($._statements, $.body))), 'end')),
      ),

    // A bodiless `fn greet()` / `def area() -> Float` in an interface.
    method_signature: ($) =>
      prec.right(
        seq(
          choice('def', 'fn'),
          field('name', $._method_name),
          optional(field('parameters', $.parameters)),
          optional(seq('->', field('return_type', $._type))),
        ),
      ),

    static_block: ($) => seq('static', $.brace_body),

    enum_declaration: ($) =>
      seq(
        'enum',
        field('name', $.constant),
        choice(
          seq('{', optional($._enum_items), '}'),
          seq(optional($._enum_items), 'end'),
        ),
      ),

    _enum_items: ($) => repeat1(choice($.enum_variant, $.function_declaration, ',', $._terminator)),

    enum_variant: ($) => seq(field('name', $.constant), optional($.parameters), optional(seq('=', $._expression))),

    // ------------------------------------------------------------------- types

    _type: ($) =>
      choice(
        $.type_identifier,
        $.generic_type,
        $.optional_type,
        $.union_type,
        $.function_type,
        $.array_type,
        $.parenthesized_type,
        alias('nil', $.type_identifier),
        alias('null', $.type_identifier),
      ),

    type_identifier: ($) => $._class_name,
    generic_type: ($) => prec(1, seq($.type_identifier, $.type_arguments)),
    type_arguments: ($) => seq('<', commaSep1($._type), '>'),
    type_parameters: ($) => seq('<', commaSep1($.constant), '>'),
    optional_type: ($) => prec(2, seq($._type, token.immediate('?'))),
    union_type: ($) => prec.left(seq($._type, '|', $._type)),
    array_type: ($) => prec(2, seq($._type, token.immediate('['), ']')),
    parenthesized_type: ($) => seq('(', $._type, ')'),
    function_type: ($) => prec.right(seq('(', commaSep($._type), ')', '->', $._type)),

    // ------------------------------------------------------------- control flow

    if_expression: ($) =>
      choice(
        prec.right(
          seq(
            'if',
            field('condition', $._expression),
            field('consequence', $.brace_body),
            optional(field('alternative', alias($._brace_else, $.else_clause))),
          ),
        ),
        seq(
          'if',
          field('condition', $._expression),
          $._line_header,
          optional(field('consequence', alias($._statements, $.body))),
          repeat($.elsif_clause),
          optional($.else_clause),
          'end',
        ),
        seq(
          'if',
          field('condition', $.parenthesized_expression),
          field('consequence', alias($._statements, $.body)),
          repeat($.elsif_clause),
          optional($.else_clause),
          'end',
        ),
      ),

    _brace_else: ($) =>
      choice(
        seq('else', choice($.brace_body, $.if_expression)),
        seq('elsif', field('condition', $._expression), field('consequence', $.brace_body), optional($._brace_else)),
      ),

    // What ends an `if` / `elsif` condition before a body on its own line:
    // `then`, a line break, or nothing after a parenthesized condition
    // (`if (x) return 1;`).
    _line_header: ($) => choice('then', $._terminator),

    elsif_clause: ($) =>
      seq(
        'elsif',
        field('condition', $._expression),
        $._line_header,

        optional(field('consequence', alias($._statements, $.body))),
      ),

    else_clause: ($) => seq('else', optional(alias($._statements, $.body))),

    unless_expression: ($) =>
      choice(
        prec.right(
          seq('unless', field('condition', $._expression), field('consequence', $.brace_body), optional(alias($._brace_else, $.else_clause))),
        ),

        seq(
          'unless',
          field('condition', $._expression),
          choice('then', $._terminator),
          optional(field('consequence', alias($._statements, $.body))),
          optional($.else_clause),
          'end',
        ),
      ),

    while_statement: ($) =>
      seq(
        choice('while', 'until'),
        field('condition', $._expression),
        choice(field('body', $.brace_body), seq(choice('do', $._terminator), optional(field('body', alias($._statements, $.body))), 'end')),
      ),

    for_statement: ($) =>
      seq(
        'for',
        choice(
          seq('(', commaSep1($._for_target), 'in', $._expression, ')'),
          seq(commaSep1($._for_target), 'in', field('iterable', $._expression)),
        ),
        choice(field('body', $.brace_body), seq(choice('do', $._terminator), optional(field('body', alias($._statements, $.body))), 'end')),
      ),

    _for_target: ($) => choice($.identifier, seq('[', commaSep1($.identifier), ']')),


    match_expression: ($) =>
      choice(
        seq('match', field('value', $._expression), '{', repeat(choice($.match_arm, ',', $._line_break)), '}'),
        seq('match', field('value', $._expression), $._line_break, repeat(choice($.match_arm, ',', $._line_break)), 'end'),
      ),

    match_arm: ($) =>
      prec.right(
        seq(
          field('pattern', $._pattern),
          optional(seq('if', field('guard', $._expression))),
          '=>',
          field('value', choice($.brace_body, $._expression)),
        ),
      ),

    _pattern: ($) => prec.left(choice($._expression, $.typed_pattern, seq($._pattern, '|', $._pattern))),

    // `Int: n` — matches an Int and binds it.
    typed_pattern: ($) => seq(field('type', $.constant), token.immediate(':'), field('name', $.identifier)),

    try_expression: ($) =>
      choice(
        prec.right(
          seq(
            'try',
            field('body', $.brace_body),
            repeat(alias($._brace_catch, $.catch_clause)),
            optional(alias($._brace_finally, $.finally_clause)),
          ),
        ),
        seq(
          'try',
          optional(field('body', alias($._statements, $.body))),
          repeat($.catch_clause),
          optional($.finally_clause),
          'end',
        ),
        seq(
          'begin',
          optional(field('body', alias($._statements, $.body))),
          repeat($.rescue_clause),
          optional($.ensure_clause),
          'end',
        ),
      ),

    _catch_header: ($) =>
      choice(
        seq('(', optional($._class_name), optional($.identifier), ')'),
        seq(commaSep1($._class_name), '=>', $.identifier),
        seq($._class_name, $.identifier),
        $._class_name,
        $.identifier,
      ),

    _brace_catch: ($) => seq('catch', optional($._catch_header), $.brace_body),
    _brace_finally: ($) => seq('finally', $.brace_body),

    catch_clause: ($) =>
      seq('catch', optional($._catch_header), $._terminator, optional(alias($._statements, $.body))),

    finally_clause: ($) => seq('finally', optional(alias($._statements, $.body))),

    rescue_clause: ($) =>
      seq(
        'rescue',
        optional($._catch_header),
        $._terminator,
        optional(alias($._statements, $.body)),
      ),


    ensure_clause: ($) => seq('ensure', optional(alias($._statements, $.body))),

    // ------------------------------------------------------------- expressions

    _expression: ($) =>
      choice(
        $.assignment,
        $.operator_assignment,
        $.binary_expression,
        $.unary_expression,
        $.ternary_expression,
        $.rescue_modifier,
        $.lambda,
        $.update_expression,
        $._primary,
      ),

    assignment: ($) =>
      prec.right(PREC.ASSIGN, seq(field('left', $._assignable), '=', field('right', $._value))),

    operator_assignment: ($) =>
      prec.right(
        PREC.ASSIGN,
        seq(
          field('left', $._assignable),
          field('operator', choice('+=', '-=', '*=', '/=', '%=', '||=', '&&=', '??=', '<<=')),
          field('right', $._expression),
        ),
      ),

    _assignable: ($) =>
      choice($.identifier, $.instance_variable, $.constant, $.member_expression, $.index_expression, $.array),

    rescue_modifier: ($) => prec.left(PREC.RESCUE, seq($._expression, 'rescue', $._expression)),

    ternary_expression: ($) =>
      prec.right(
        PREC.TERNARY,
        seq(field('condition', $._expression), '?', field('consequence', $._expression), ':', field('alternative', $._expression)),
      ),

    binary_expression: ($) => {
      const table = [
        [PREC.NULLISH, '??'],
        [PREC.OR, choice('||', 'or')],
        [PREC.AND, choice('&&', 'and')],
        [PREC.EQUALITY, choice('==', '!=', '===', '=~')],
        [PREC.COMPARE, choice('<', '<=', '>', '>=', '<=>', 'in')],
        [PREC.RANGE, choice('..', '..=')],
        [PREC.PIPELINE, '|>'],
        [PREC.SHIFT, choice('<<', '>>')],
        [PREC.ADD, choice('+', '-')],
        [PREC.MULTIPLY, choice('*', '/', '%', '**')],
      ];
      return choice(
        ...table.map(([precedence, operator]) =>
          prec.left(
            /** @type {number} */ (precedence),
            seq(field('left', $._expression), field('operator', /** @type {RuleOrLiteral} */ (operator)), field('right', $._expression)),
          ),
        ),
      );
    },

    unary_expression: ($) =>
      choice(
        prec(PREC.UNARY, seq(field('operator', choice('-', '!', '~', '+')), field('operand', $._expression))),
        prec(PREC.NOT, seq(field('operator', 'not'), field('operand', $._expression))),
      ),

    update_expression: ($) => prec.left(PREC.POSTFIX, seq($._assignable, choice('++', '--'))),

    lambda: ($) =>
      prec.right(
        PREC.LAMBDA,
        choice(
          seq('fn', field('parameters', $.parameters), optional(seq('->', $._type)), field('body', choice($.brace_body, $._expression))),
          seq(field('parameters', $.lambda_parameters), field('body', choice($.brace_body, $._expression))),
          seq('->', field('parameters', $.parameters), field('body', $.brace_body)),
        ),
      ),

    lambda_parameters: ($) => seq('|', commaSep(alias($._bar_parameter, $.parameter)), '|'),

    // Between `|…|` a parameter's type is no union: the `|` closes the
    // parameters, so a union there is written in parentheses.
    _bar_parameter: ($) =>
      seq(
        optional(choice('...', '*', '&')),
        field('name', $.identifier),
        optional(seq(':', field('type', $._bar_type))),
      ),

    _bar_type: ($) =>
      choice(
        $.type_identifier,
        $.generic_type,
        alias($._bar_optional_type, $.optional_type),
        alias($._bar_array_type, $.array_type),
        $.parenthesized_type,
        $.function_type,
      ),
    _bar_optional_type: ($) => prec(2, seq($._bar_type, token.immediate('?'))),
    _bar_array_type: ($) => prec(2, seq($._bar_type, token.immediate('['), ']')),

    _primary: ($) =>
      choice(
        $.identifier,
        $.constant,
        $.instance_variable,
        $.self,
        $.super,
        $.nil,
        $.true,
        $.false,
        $.integer,
        $.float,
        $.string,
        $.raw_string,
        $.raw_string_triple,
        $.command_string,
        $.simple_symbol,
        $.word_array,
        $.sdbql_block,
        $.array,
        $.hash,
        $.parenthesized_expression,
        $.member_expression,
        $.scope_resolution,
        $.index_expression,
        $.call,
        $.if_expression,
        $.unless_expression,
        $.match_expression,
        $.try_expression,
        $.spread,
        $.new_expression,
        $.comprehension,
      ),

    // `new Point(1, 2)`: the keyword form of `Point.new(1, 2)`.
    new_expression: ($) => prec.right(PREC.CALL, seq('new', field('class', $._class_name), optional(field('arguments', $.arguments)))),

    // `[x * x for x in xs if x > 1]`
    comprehension: ($) =>
      seq(
        '[',
        field('body', $._expression),
        'for',
        commaSep1($._for_target),
        'in',
        field('iterable', $._expression),
        optional(seq('if', field('condition', $._expression))),
        ']',
      ),

    parenthesized_expression: ($) => seq('(', optional($._statements), ')'),

    spread: ($) => prec(PREC.UNARY, seq('...', $._expression)),

    member_expression: ($) =>
      prec(
        PREC.MEMBER,
        seq(field('object', $._primary), field('operator', choice('.', '&.')), field('property', $._member_name)),
      ),

    _member_name: ($) =>
      choice(
        alias($.identifier, $.property_identifier),
        alias($._predicate_name, $.property_identifier),
        alias($.constant, $.property_identifier),
        alias(choice('new', 'class', 'end', 'in', 'not', 'match', 'if', 'unless', 'then', 'do', 'self', 'nil', 'when', 'case', 'import', 'export', 'try', 'catch', 'fn', 'def', 'next', 'break', 'return', 'and', 'or', 'interface', 'enum', 'module', 'static', 'private', 'public', 'protected', 'const', 'let', 'while', 'for', 'super', 'true', 'false', 'null', 'throw', 'rescue', 'ensure', 'begin', 'finally', 'else', 'elsif', 'from', 'as', 'extends', 'implements', 'until'), $.property_identifier),
      ),

    _predicate_name: (_) => /[a-zA-Z_][a-zA-Z0-9_]*[?!]/,

    scope_resolution: ($) => prec(PREC.MEMBER, seq(optional(field('scope', $._primary)), '::', field('name', choice($.constant, $.identifier)))),

    index_expression: ($) =>
      prec(PREC.MEMBER, seq(field('object', $._primary), token.immediate('['), commaSep1($._expression), optional(','), ']')),

    call: ($) =>
      choice(
        prec(
          PREC.CALL,
          seq(field('function', choice($._primary, alias($._predicate_name, $.identifier))), field('arguments', $.arguments)),
        ),
        // A trailing block. After a condition (`if x.any? { … }`, `match kind {`)
        // the `{` may instead open the body: both readings are kept and the
        // one that parses wins, a block with `|params|` over a body over a
        // block without.
        seq(
          field('function', choice($.identifier, $.member_expression, alias($._predicate_name, $.identifier), $.call)),
          field('block', $._block),
        ),
      ),

    arguments: ($) => seq(token.immediate('('), listOf($._argument), ')'),

    _argument: ($) => choice($._expression, $.keyword_argument, $.block_argument),

    keyword_argument: ($) =>
      seq(field('name', choice($.identifier, $.constant, $.string)), token.immediate(':'), field('value', $._expression)),

    block_argument: ($) => seq('&', $._expression),

    // `has_many :comments`, `before_save :normalize, on: "create"`: a call
    // with no parentheses, at the start of a statement.
    command_call: ($) =>
      prec.right(
        seq(
          field('function', $.identifier),
          field('arguments', alias($.command_arguments, $.arguments)),
          optional(field('block', $.do_block)),
        ),
      ),

    command_arguments: ($) =>
      prec.right(
        seq(
          choice(
            $.simple_symbol,
            $.string,
            $.integer,
            $.float,
            $.raw_string,
            $.constant,
            $.keyword_argument,
            $.new_expression,
            $.array,
            $.true,
            $.false,
            $.nil,
          ),
          repeat(seq(',', choice($._expression, $.keyword_argument))),
        ),
      ),

    _block: ($) => choice($.block, $.do_block),

    block: ($) =>
      choice(
        prec.dynamic(2, seq('{', $.block_parameters, optional($._statements), '}')),
        prec.dynamic(-1, seq('{', optional($._statements), '}')),
      ),

    do_block: ($) => seq('do', optional($.block_parameters), optional(alias($._statements, $.body)), 'end'),

    block_parameters: ($) => seq('|', commaSep(choice(alias($._bar_parameter, $.parameter), $.destructuring_parameter)), '|'),


    destructuring_parameter: ($) => seq('(', commaSep1($.identifier), ')'),

    array: ($) => seq('[', listOf($._expression), ']'),

    hash: ($) =>
      choice(
        seq('{', listOf(choice($.pair, $.spread)), '}'),
        // `{k: v for x in xs}`
        seq(
          '{',
          $.pair,
          'for',
          commaSep1($._for_target),
          'in',
          field('iterable', $._expression),
          optional(seq('if', field('condition', $._expression))),
          '}',
        ),
      ),

    pair: ($) =>
      choice(
        seq(
          field('key', choice($.hash_key_symbol, $.string, $.integer, $.true, $.false, $.nil, $.simple_symbol)),
          token.immediate(':'),
          field('value', $._expression),
        ),
        seq(field('key', $._expression), '=>', field('value', $._expression)),
      ),

    hash_key_symbol: ($) => choice($.identifier, $.constant, alias($._predicate_name, $.identifier)),

    // ---------------------------------------------------------------- literals

    identifier: (_) => /[a-z_][a-zA-Z0-9_]*/,
    constant: (_) => /[A-Z][a-zA-Z0-9_]*/,
    instance_variable: (_) => /@[a-zA-Z_][a-zA-Z0-9_]*[?!]?/,

    self: (_) => choice('self', 'this'),
    super: (_) => 'super',
    nil: (_) => choice('nil', 'null'),
    true: (_) => 'true',
    false: (_) => 'false',

    integer: (_) => /\d[\d_]*/,
    float: (_) => token(choice(/\d[\d_]*\.\d[\d_]*([eE][+-]?\d+)?[dD]?/, /\d[\d_]*[eE][+-]?\d+/, /\d[\d_]*[dD]/)),

    string: ($) =>
      choice(
        seq('"', repeat(choice(alias($._dq_string_content, $.string_content), $.interpolation)), '"'),
        seq("'", repeat(choice(alias($._sq_string_content, $.string_content), $.interpolation)), "'"),
      ),

    interpolation: ($) => seq(alias($._interpolation_start, '#{'), optional($._statements), '}'),

    raw_string: (_) => token(choice(/r"[^"]*"/, /r'[^']*'/)),
    command_string: (_) => /`[^`]*`/,
    simple_symbol: (_) => /:[a-zA-Z_][a-zA-Z0-9_]*[?!]?/,
    word_array: (_) => /%[win]\[[^\]]*\]/,

    comment: (_) => token(choice(seq('#', /.*/), seq('//', /.*/))),
  },
});
