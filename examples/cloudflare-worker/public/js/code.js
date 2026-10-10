// Syntax colours for the code samples, with highlight.js (loaded before this
// file). Blocks say their language: <pre><code class="language-soli|toml|bash">.
//
// Instant navigation (/__soli/nav.js) swaps <body> without running this file
// again, so the work is in an idempotent init(), called on the first load and
// on every `soli:load`; highlight.js marks what it did (data-highlighted) and
// those blocks are skipped.
(function () {
  var registered = false;

  // Soli for highlight.js — the grammar the Soli docs site uses.
  function registerSoli() {
    if (registered || typeof hljs === "undefined") return;
    registered = true;
    hljs.registerLanguage("soli", function (hljs) {
      var IDENT = "[A-Za-z_][A-Za-z0-9_]*[?!]?";
      var KEYWORDS = {
        keyword: "def fn if unless then else elsif end return let for in while do break next class " +
          "interface module enum static this super new import as try catch rescue ensure finally " +
          "throw yield match case when and or not export private const",
        literal: "nil null true false",
        built_in: "print puts len str int float range render render_json redirect grouped params " +
          "getenv clock Model Controller DateTime Crypto HTTP JSON"
      };
      var STRING = {
        className: "string",
        variants: [
          { begin: '"""', end: '"""+' },
          { begin: 'r?"', end: '"', contains: [hljs.BACKSLASH_ESCAPE,
            { className: "subst", begin: "#\\{", end: "\\}", keywords: KEYWORDS }] }
        ]
      };
      return {
        name: "Soli",
        keywords: KEYWORDS,
        contains: [
          hljs.COMMENT("#", "$"),
          STRING,
          { className: "number", begin: "-?\\b\\d+(\\.\\d+)?\\b", relevance: 0 },
          { className: "symbol", begin: ":" + IDENT, relevance: 0 },
          { className: "variable", begin: "@" + IDENT, relevance: 0 },
          {
            beginKeywords: "class interface module enum",
            end: "($|\\{|\\bend\\b|\\n)",
            returnEnd: true,
            contains: [
              { className: "title.class", begin: IDENT, relevance: 0 },
              { begin: "<", contains: [{ className: "title.class.inherited", begin: IDENT }] }
            ]
          },
          { className: "title.function", begin: "(?:def|fn)\\s+" + IDENT, relevance: 10 },
          { className: "title.function", begin: IDENT + "(?=\\s*\\()", relevance: 0 }
        ]
      };
    });
  }

  function init() {
    if (typeof hljs === "undefined") return;
    registerSoli();
    var blocks = document.querySelectorAll('pre code[class*="language-"]:not([data-highlighted])');
    for (var i = 0; i < blocks.length; i++) hljs.highlightElement(blocks[i]);
  }

  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", init);
  } else {
    init();
  }
  document.addEventListener("soli:load", init);
})();
