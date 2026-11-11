; `.slv` views, parsed by the `embedded_template` grammar (the one ERB and
; EJS use). The markup and the code are injected: see injections.scm.

(comment_directive) @comment

[
  "<%"
  "<%="
  "<%-"
  "<%#"
  "%>"
  "-%>"
] @keyword
