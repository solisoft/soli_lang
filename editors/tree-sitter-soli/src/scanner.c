// External scanner for tree-sitter-soli.
//
// Soli's own lexer drops every newline and lets the parser end a statement at
// a line break. Here a line break is a token only where the grammar can end a
// statement (`valid_symbols[LINE_BREAK]`); anywhere else — inside `( )` or
// `[ ]`, after a binary operator — it is plain whitespace. A line that starts
// with `.`, `&.`, `|>`, `&&`, `||` or `??` continues the one above.
//
// In a template (`.slv`, injected tag by tag) each tag's code starts a
// statement, as a line would.
//
// It also reads the inside of strings (so `#` there is not a comment and
// `#{` opens an interpolation), `"""` raw strings, `@sdbql{ … }` blocks and
// nested `/* */` comments.

#include "tree_sitter/parser.h"

#include <stdbool.h>
#include <stdint.h>
#include <stdlib.h>

enum TokenType {
  LINE_BREAK,
  DQ_STRING_CONTENT,
  SQ_STRING_CONTENT,
  INTERPOLATION_START,
  TRIPLE_STRING,
  SDBQL_BLOCK,
  BLOCK_COMMENT,
  ERROR_SENTINEL,
};

// In a template the code of each `<% %>` tag is an included range of one
// combined tree, and the markup between tags — newlines included — is never
// seen. A tag therefore starts a statement: the scanner emits an empty line
// break at the start of each range, once (`range_break_emitted` is cleared
// as soon as the scanner runs anywhere else).
typedef struct {
  bool range_break_emitted;
} Scanner;

void *tree_sitter_soli_external_scanner_create(void) { return calloc(1, sizeof(Scanner)); }
void tree_sitter_soli_external_scanner_destroy(void *payload) { free(payload); }

unsigned tree_sitter_soli_external_scanner_serialize(void *payload, char *buffer) {
  buffer[0] = ((Scanner *)payload)->range_break_emitted;
  return 1;
}

void tree_sitter_soli_external_scanner_deserialize(void *payload, const char *buffer, unsigned length) {
  ((Scanner *)payload)->range_break_emitted = length > 0 && buffer[0];
}

static inline void advance(TSLexer *lexer) { lexer->advance(lexer, false); }
static inline void skip(TSLexer *lexer) { lexer->advance(lexer, true); }

// The inside of a string up to its closing quote, an `#{`, or the end of
// input. Escapes are kept whole, so `\"` does not close it.
static bool scan_string_content(TSLexer *lexer, int32_t quote, enum TokenType symbol, bool has_content) {
  for (;;) {
    int32_t c = lexer->lookahead;
    if (c == 0 && lexer->eof(lexer)) break;
    if (c == quote) break;
    if (c == '#') {
      lexer->mark_end(lexer);
      advance(lexer);
      if (lexer->lookahead == '{') {
        // Stop before the interpolation.
        if (has_content) {
          lexer->result_symbol = symbol;
          return true;
        }
        return false;
      }
      has_content = true;
      continue;
    }
    if (c == '\\') {
      advance(lexer);
      if (!(lexer->lookahead == 0 && lexer->eof(lexer))) advance(lexer);
      has_content = true;
      continue;
    }
    advance(lexer);
    has_content = true;
  }
  lexer->mark_end(lexer);
  if (!has_content) return false;
  lexer->result_symbol = symbol;
  return true;
}

// `"""` … `"""`, raw. The opening quote is the lookahead.
static bool scan_triple_string(TSLexer *lexer) {
  for (int i = 0; i < 3; i++) {
    if (lexer->lookahead != '"') return false;
    advance(lexer);
  }
  int quotes = 0;
  while (!(lexer->lookahead == 0 && lexer->eof(lexer))) {
    if (lexer->lookahead == '"') {
      quotes++;
      advance(lexer);
      if (quotes == 3) {
        // A run of more than three closes on its last three.
        while (lexer->lookahead == '"') advance(lexer);
        lexer->mark_end(lexer);
        lexer->result_symbol = TRIPLE_STRING;
        return true;
      }
    } else {
      quotes = 0;
      advance(lexer);
    }
  }
  return false;
}

static bool consume(TSLexer *lexer, const char *word) {
  for (const char *p = word; *p; p++) {
    if (lexer->lookahead != *p) return false;
    advance(lexer);
  }
  return true;
}

// `@sdbql{ … }` or the legacy `@sdql{ … }`, braces balanced. The `@` is the
// lookahead.
static bool scan_sdbql_block(TSLexer *lexer) {
  advance(lexer);  // @
  if (!consume(lexer, "sd")) return false;
  if (lexer->lookahead == 'b') advance(lexer);
  if (!consume(lexer, "ql{")) return false;
  int depth = 1;
  while (depth > 0) {
    if (lexer->lookahead == 0 && lexer->eof(lexer)) return false;
    if (lexer->lookahead == '{') depth++;
    if (lexer->lookahead == '}') depth--;
    advance(lexer);
  }
  lexer->mark_end(lexer);
  lexer->result_symbol = SDBQL_BLOCK;
  return true;
}

// `/* … */`, nesting. The `/` is the lookahead.
static bool scan_block_comment(TSLexer *lexer) {
  advance(lexer);
  if (lexer->lookahead != '*') return false;
  advance(lexer);
  int depth = 1;
  while (depth > 0) {
    if (lexer->lookahead == 0 && lexer->eof(lexer)) break;
    if (lexer->lookahead == '*') {
      advance(lexer);
      if (lexer->lookahead == '/') {
        advance(lexer);
        depth--;
      }
    } else if (lexer->lookahead == '/') {
      advance(lexer);
      if (lexer->lookahead == '*') {
        advance(lexer);
        depth++;
      }
    } else {
      advance(lexer);
    }
  }
  lexer->mark_end(lexer);
  lexer->result_symbol = BLOCK_COMMENT;
  return true;
}

// After a newline (consumed, end marked): whether the next code line carries
// on the expression above instead of starting a statement.
static bool continues_previous_line(TSLexer *lexer) {
  for (;;) {
    int32_t c = lexer->lookahead;
    if (c == ' ' || c == '\t' || c == '\r' || c == '\n') {
      skip(lexer);
      continue;
    }
    break;
  }
  switch (lexer->lookahead) {
    case '.':
      advance(lexer);
      return lexer->lookahead != '.';
    case '&':
      advance(lexer);
      return lexer->lookahead == '.' || lexer->lookahead == '&';
    case '|':
      advance(lexer);
      return lexer->lookahead == '>' || lexer->lookahead == '|';
    case '?':
      // `?? fallback`, or the `? then` of a ternary split over lines.
      advance(lexer);
      return lexer->lookahead == '?' || lexer->lookahead == ' ' || lexer->lookahead == '\t';
    case ':':
      // The `: else` of a ternary (a `:symbol` would be glued to its name).
      advance(lexer);
      return lexer->lookahead == ' ' || lexer->lookahead == '\t';
    default:
      return false;
  }
}

bool tree_sitter_soli_external_scanner_scan(void *payload, TSLexer *lexer, const bool *valid_symbols) {
  Scanner *scanner = (Scanner *)payload;
  // Error recovery marks every symbol valid: let the internal lexer resync.
  if (valid_symbols[ERROR_SENTINEL]) return false;

  if (!lexer->is_at_included_range_start(lexer)) scanner->range_break_emitted = false;

  if (valid_symbols[INTERPOLATION_START] && lexer->lookahead == '#') {
    if (valid_symbols[DQ_STRING_CONTENT] || valid_symbols[SQ_STRING_CONTENT]) {
      lexer->mark_end(lexer);
      advance(lexer);
      if (lexer->lookahead == '{') {
        advance(lexer);
        lexer->mark_end(lexer);
        lexer->result_symbol = INTERPOLATION_START;
        return true;
      }
      // A lone `#` is string content.
      if (valid_symbols[DQ_STRING_CONTENT]) return scan_string_content(lexer, '"', DQ_STRING_CONTENT, true);
      return scan_string_content(lexer, '\'', SQ_STRING_CONTENT, true);
    }
  }
  if (valid_symbols[DQ_STRING_CONTENT]) return scan_string_content(lexer, '"', DQ_STRING_CONTENT, false);
  if (valid_symbols[SQ_STRING_CONTENT]) return scan_string_content(lexer, '\'', SQ_STRING_CONTENT, false);

  for (;;) {
    // The whitespace may carry the lexer into the next tag's range; the
    // check holds only on the range's first byte.
    if (lexer->is_at_included_range_start(lexer) && valid_symbols[LINE_BREAK] && !scanner->range_break_emitted) {
      scanner->range_break_emitted = true;
      lexer->mark_end(lexer);
      lexer->result_symbol = LINE_BREAK;
      return true;
    }
    if (lexer->lookahead == ' ' || lexer->lookahead == '\t' || lexer->lookahead == '\r') {
      skip(lexer);
      continue;
    }
    break;
  }

  if (lexer->lookahead == '\n' && valid_symbols[LINE_BREAK]) {
    advance(lexer);
    lexer->mark_end(lexer);
    if (continues_previous_line(lexer)) return false;
    lexer->result_symbol = LINE_BREAK;
    return true;
  }
  if (lexer->lookahead == ';' && valid_symbols[LINE_BREAK]) {
    advance(lexer);
    lexer->mark_end(lexer);
    lexer->result_symbol = LINE_BREAK;
    return true;
  }
  if (lexer->lookahead == '"' && valid_symbols[TRIPLE_STRING]) return scan_triple_string(lexer);
  if (lexer->lookahead == '@' && valid_symbols[SDBQL_BLOCK]) return scan_sdbql_block(lexer);
  if (lexer->lookahead == '/' && valid_symbols[BLOCK_COMMENT]) return scan_block_comment(lexer);
  return false;
}
