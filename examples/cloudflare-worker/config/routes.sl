# Soli on Cloudflare Workers — the site at cf.solisoft.net, and the example
# `soli edge build` is documented with. Every page is computed by the Soli
# interpreter, compiled to wasm, inside the Worker; /d1 keeps its data in D1.

get("/", "pages#index")
get("/guide", "pages#guide")
get("/limits", "pages#limits")
get("/try", "playground#show")
get("/d1", "check_ins#index")
post("/d1/check-in", "check_ins#create")
get("/api/info", "api#info")
