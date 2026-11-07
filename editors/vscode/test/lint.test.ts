import * as assert from "node:assert/strict";
import { test } from "node:test";
import { fallbackLintActive, lacksLanguageServer, parseLintOutput } from "../src/lint";

test("soli lint findings are parsed, 0-based", () => {
  const output = [
    "app/controllers/posts_controller.sl:4:19 - [security/unfiltered-mass-assignment] Post.create with raw params",
    "    4 | Post.create(params)",
    "",
    "1 issue(s) found in 1 file(s)",
  ].join("\n");
  assert.deepEqual(parseLintOutput(output), [
    {
      line: 3,
      column: 18,
      rule: "security/unfiltered-mass-assignment",
      message: "Post.create with raw params",
    },
  ]);
});

test("Windows paths with a drive colon parse too", () => {
  const [finding] = parseLintOutput("C:\\app\\x.sl:2:1 - [naming/snake-case] bad name\r\n");
  assert.equal(finding.line, 1);
  assert.equal(finding.rule, "naming/snake-case");
});

test("a soli built without the lsp feature is recognised and not restarted", () => {
  const refusal = "This build of soli has no language server: it was built without the `lsp` feature.\n";
  assert.equal(lacksLanguageServer(1, refusal), true);
  assert.equal(lacksLanguageServer(null, refusal), true);
});

test("a crash is not mistaken for a missing language server", () => {
  assert.equal(lacksLanguageServer(101, "thread 'main' panicked at src/lsp/mod.rs"), false);
  assert.equal(lacksLanguageServer(0, ""), false);
});

test("the fallback lint runs only while the server does not", () => {
  assert.equal(fallbackLintActive({ lintEnabled: true, serverActive: true }), false);
  assert.equal(fallbackLintActive({ lintEnabled: true, serverActive: false }), true);
  assert.equal(fallbackLintActive({ lintEnabled: false, serverActive: false }), false);
});
