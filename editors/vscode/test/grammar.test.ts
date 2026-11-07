// Tokenizes snippets with the grammars the extension ships, through the same
// engine VS Code uses (vscode-textmate + Oniguruma).

import * as assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import * as path from "node:path";
import { before, test } from "node:test";

// eslint-disable-next-line @typescript-eslint/no-require-imports
const textmate = require("vscode-textmate");
// eslint-disable-next-line @typescript-eslint/no-require-imports
const oniguruma = require("vscode-oniguruma");

const syntaxes = path.resolve(__dirname, "..", "syntaxes");

// text.html.basic ships with the editor, not with this extension: a small
// stand-in with tags and quoted attribute values is enough to check the
// template grammar's injections.
const htmlStub = {
  scopeName: "text.html.basic",
  patterns: [
    {
      name: "meta.tag.html",
      begin: "</?[A-Za-z][\\w-]*",
      end: "/?>",
      patterns: [{ name: "string.quoted.double.html", begin: '"', end: '"' }],
    },
  ],
};

let registry: any;

before(async () => {
  const wasm = readFileSync(require.resolve("vscode-oniguruma/release/onig.wasm"));
  await oniguruma.loadWASM(wasm.buffer.slice(wasm.byteOffset, wasm.byteOffset + wasm.byteLength));
  registry = new textmate.Registry({
    onigLib: Promise.resolve({
      createOnigScanner: (patterns: string[]) => new oniguruma.OnigScanner(patterns),
      createOnigString: (text: string) => new oniguruma.OnigString(text),
    }),
    loadGrammar: async (scopeName: string) => {
      const files: Record<string, string> = {
        "source.soli": "soli.tmLanguage.json",
        "text.html.soli": "soli-template.tmLanguage.json",
      };
      if (scopeName === "text.html.basic") {
        return textmate.parseRawGrammar(JSON.stringify(htmlStub), "html.json");
      }
      const file = files[scopeName];
      if (!file) {
        return null;
      }
      const fullPath = path.join(syntaxes, file);
      return textmate.parseRawGrammar(readFileSync(fullPath, "utf8"), fullPath);
    },
  });
});

/** Each token's text with its innermost scope, for a snippet. */
async function tokenize(scope: string, source: string): Promise<Array<[string, string[]]>> {
  const grammar = await registry.loadGrammar(scope);
  let state = textmate.INITIAL;
  const tokens: Array<[string, string[]]> = [];
  for (const line of source.split("\n")) {
    const result = grammar.tokenizeLine(line, state);
    for (const token of result.tokens) {
      tokens.push([line.slice(token.startIndex, token.endIndex), token.scopes]);
    }
    state = result.ruleStack;
  }
  return tokens;
}

/** The scopes of the `nth` token whose text is `text` — or, when no token is
 * exactly that, contains it (a word that gets no scope of its own is merged
 * with its neighbours). */
async function scopesOf(scope: string, source: string, text: string, nth = 0): Promise<string[]> {
  const tokens = await tokenize(scope, source);
  let matches = tokens.filter(([tokenText]) => tokenText === text);
  if (matches.length === 0) {
    matches = tokens.filter(([tokenText]) => tokenText.includes(text));
  }
  assert.ok(matches.length > nth, `no token ${JSON.stringify(text)} in ${JSON.stringify(source)}`);
  return matches[nth][1];
}

function hasScope(scopes: string[], prefix: string): boolean {
  return scopes.some((scope) => scope === prefix || scope.startsWith(prefix + "."));
}

const soli = (source: string, text: string, nth = 0) => scopesOf("source.soli", source, text, nth);

test("the lexer's keywords are keywords", async () => {
  for (const word of ["module", "enum", "begin", "ensure", "rescue", "then", "do", "unless", "elsif"]) {
    assert.ok(hasScope(await soli(`${word} x`, word), "keyword"), word);
  }
  assert.ok(hasScope(await soli("x = a and not b or c", "and"), "keyword.operator.word"));
  assert.ok(hasScope(await soli("x = nil", "nil"), "constant.language"));
  assert.ok(hasScope(await soli("self.name", "self"), "variable.language"));
  assert.ok(hasScope(await soli("x: Decimal = 1.5D", "Decimal"), "storage.type"));
});

test("words the lexer does not reserve are not keywords", async () => {
  for (const word of ["async", "await", "Any"]) {
    assert.ok(!hasScope(await soli(`${word} x`, word), "keyword"), word);
    assert.ok(!hasScope(await soli(`${word} x`, word), "storage.type"), word);
  }
});

test("a method named like a keyword, or with a ? / ! suffix, is not a keyword", async () => {
  assert.ok(!hasScope(await soli("user.class", "class"), "keyword"));
  assert.ok(hasScope(await soli("xs.empty?", "empty?"), "entity.name.function"));
  assert.ok(!hasScope(await soli("if done? then x end", "done?"), "keyword"));
});

test("interpolation is code inside a string", async () => {
  const source = 'greeting = "Hello #{user.name}!"';
  assert.ok(hasScope(await soli(source, "#{"), "punctuation.section.interpolation.begin"));
  const name = await soli(source, "name");
  assert.ok(hasScope(name, "meta.interpolation"));
  assert.ok(hasScope(name, "entity.name.function.member"));
  assert.ok(hasScope(await soli(source, "!"), "string.quoted.double"));
});

test("a hash inside an interpolation does not close it", async () => {
  const source = '"#{ {a: 1}.keys } done"';
  assert.ok(hasScope(await soli(source, " done"), "string.quoted.double"));
  assert.ok(!hasScope(await soli(source, " done"), "meta.interpolation"));
});

test("triple-quoted, raw and backtick strings", async () => {
  const triple = await tokenize("source.soli", 'x = """\nraw \\n #{no}\n"""\ny = 1');
  const body = triple.find(([text]) => text.includes("raw"));
  assert.ok(body && hasScope(body[1], "string.quoted.triple"));
  assert.ok(!triple.some(([, scopes]) => hasScope(scopes, "meta.interpolation")));
  assert.ok(hasScope(await soli('p = r"C:\\Users\\name"', "C:\\Users\\name"), "string.quoted.double.raw"));
  assert.ok(hasScope(await soli("out = `ls -la`", "ls -la"), "string.interpolated.backtick"));
});

test("escapes, valid and not", async () => {
  assert.ok(hasScope(await soli('"a\\e[1m"', "\\e"), "constant.character.escape"));
  assert.ok(hasScope(await soli('"\\u{1F600}"', "\\u{1F600}"), "constant.character.escape"));
  assert.ok(hasScope(await soli('"\\("', "\\("), "invalid.illegal"));
});

test("@sdbql and @sdql blocks, with interpolation", async () => {
  const source = "rows = @sdbql{ FOR u IN users FILTER u.age > #{min} RETURN u }";
  assert.ok(hasScope(await soli(source, "FOR"), "keyword.other.sdbql"));
  assert.ok(hasScope(await soli(source, "min"), "meta.interpolation"));
  assert.ok(hasScope(await soli("@sdql{ RETURN 1 }", "RETURN"), "keyword.other.sdbql"));
  const after = await soli("x = @sdbql{ RETURN {a: 1} }\ny = 2", "y");
  assert.ok(!hasScope(after, "meta.embedded.sdbql"));
});

test("symbols, symbol arrays and word arrays", async () => {
  assert.ok(hasScope(await soli("emit :saved?", "saved?"), "constant.other.symbol"));
  assert.ok(hasScope(await soli("emit :saved?", ":"), "punctuation.definition.symbol"));
  assert.ok(hasScope(await soli("x = %i[a b]", "a b"), "constant.other.symbol.array"));
  assert.ok(hasScope(await soli("x = %w[a b]", "a b"), "string.quoted.other.array"));
  // A colon glued to a key is a separator, not a symbol.
  assert.ok(!hasScope(await soli('h = {"key":false}', "false"), "constant.other.symbol"));
});

test("instance variables, definitions and block parameters", async () => {
  assert.ok(hasScope(await soli("@name = name", "@"), "variable.other.readwrite.instance"));
  assert.ok(hasScope(await soli("def admin?", "admin?"), "entity.name.function"));
  const klass = "class Post < Model";
  assert.ok(hasScope(await soli(klass, "Post"), "entity.name.type.class"));
  assert.ok(hasScope(await soli(klass, "Model"), "entity.other.inherited-class"));
  assert.ok(hasScope(await soli("xs.map { |x, i| x }", "x, i"), "variable.parameter.block"));
  assert.ok(hasScope(await soli("xs.each do |x|", "x"), "variable.parameter.block"));
});

test("numbers: integers, floats and decimals", async () => {
  assert.ok(hasScope(await soli("x = 1_000", "1_000"), "constant.numeric.integer"));
  assert.ok(hasScope(await soli("x = 3.14", "3.14"), "constant.numeric.float"));
  assert.ok(hasScope(await soli("x = 19.99D", "19.99D"), "constant.numeric.decimal"));
});

test("comments: #, // and nested /* */", async () => {
  assert.ok(hasScope(await soli("x = 1 # note", " note"), "comment.line.number-sign"));
  assert.ok(hasScope(await soli("x = 1 // note", " note"), "comment.line.double-slash"));
  const nested = await tokenize("source.soli", "/* a /* b */ c */ d");
  const c = nested.find(([text]) => text.includes(" c "));
  assert.ok(c && hasScope(c[1], "comment.block"));
});

const template = (source: string, text: string, nth = 0) => scopesOf("text.html.soli", source, text, nth);

test("template tags embed Soli", async () => {
  const source = "<h1><%= @title %></h1>";
  assert.ok(hasScope(await template(source, "<%="), "punctuation.section.embedded.begin"));
  assert.ok(hasScope(await template(source, "@"), "variable.other.readwrite.instance"));
  assert.ok(hasScope(await template(source, "@"), "source.soli"));
  assert.ok(hasScope(await template(source, "%>"), "punctuation.section.embedded.end"));
});

test("<%- … -%> and a block spanning lines", async () => {
  const source = "<%- form_with(post) do |f| -%>\n  <p>x</p>\n<%- end -%>";
  assert.ok(hasScope(await template(source, "-%>"), "punctuation.section.embedded.end"));
  assert.ok(hasScope(await template(source, "f"), "variable.parameter.block"));
  assert.ok(hasScope(await template(source, "end"), "keyword.control"));
});

test("a # comment inside a tag stops at %>", async () => {
  const source = "<% # note %><b>";
  assert.ok(hasScope(await template(source, "%>"), "punctuation.section.embedded.end"));
});

test("<%# … %> is a comment and its content is not code", async () => {
  const source = "<%# if x %>after";
  assert.ok(hasScope(await template(source, " if x "), "comment.block.template"));
  assert.ok(!hasScope(await template(source, "after"), "comment"));
});

test("a tag inside an HTML attribute value is still code", async () => {
  const source = '<a href="<%= post_path(post) %>">';
  assert.ok(hasScope(await template(source, "post_path"), "source.soli"));
});
