; Markup between tags is HTML; the code of every tag is Soli, read as one
; program (`combined`), so `<% if x %> … <% end %>` is one `if`. The Soli
; grammar starts a statement at each tag, as at a line break.

((content) @injection.content
  (#set! injection.language "html")
  (#set! injection.combined))

((code) @injection.content
  (#set! injection.language "soli")
  (#set! injection.combined))
