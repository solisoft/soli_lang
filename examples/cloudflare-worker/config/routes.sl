# Soli on Cloudflare Workers — the site at cf.solisoft.net, and the example
# `soli edge build` is documented with. No database: every page is computed
# by the Soli interpreter, compiled to wasm, inside the Worker.

get("/", "pages#index")
get("/guide", "pages#guide")
get("/limits", "pages#limits")
get("/try", "playground#show")
get("/api/info", "api#info")
