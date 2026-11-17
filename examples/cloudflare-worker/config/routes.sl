# Soli on Cloudflare Workers — the site at cf.solisoft.net, and the example
# `soli edge build` is documented with. Every page is computed by the Soli
# interpreter, compiled to wasm, inside the Worker; /d1 keeps its data in D1.

get("/", "pages#index")
get("/guide", "pages#guide")
get("/limits", "pages#limits")
get("/live", "pages#live")
get("/try", "playground#show")
get("/d1", "check_ins#index")
post("/d1/check-in", "check_ins#create")
get("/invoices", "invoices#index")
post("/invoices", "invoices#create")
get("/invoices/:id", "invoices#show")
get("/invoices/:id/pdf", "invoices#pdf")
get("/invoices/:id/preview", "invoices#preview")
get("/pdf", "documents#index")
get("/pdf/pass.pdf", "documents#pass")
get("/pdf/pass.png", "documents#preview")
get("/api/info", "api#info")

# A LiveView on the edge: its socket is a Cloudflare Durable Object that
# hibernates between clicks. One shared board for every visitor (a room).
router_live("tally", "live#tally")
live_rooms("tally")

# The invoice editor: each invoice is a room named after its key, so every
# screen open on one invoice edits the same instance.
router_live("invoice_editor", "live#invoice_editor")
live_rooms("invoice_editor")
