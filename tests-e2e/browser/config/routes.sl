# Browser e2e fixture.
#
# Every page here exists to be driven by a real browser: the pages carry the
# links, forms, controls and scripts that the HTTP-level fixtures deliberately
# do without, because at that layer they would be untestable decoration.

get("/", "pages#index")
get("/about", "pages#about")
get("/form", "pages#form")
post("/form", "pages#submit")
get("/dynamic", "pages#dynamic")
get("/slow", "pages#slow")
get("/broken", "pages#broken")
get("/morph/a", "pages#morph_a")
get("/morph/b", "pages#morph_b")

# LiveView: the socket endpoint plus the page that mounts it.
router_live("counter", "live#counter")
router_live("badge", "live#badge")
router_live("about", "live#about")
router_live("uploads", "live#uploads")
get("/live", "pages#live")
get("/uploads", "pages#uploads")
get("/relative", "pages#relative")
get("/relative-fallback", "pages#relative_fallback")
get("/relative-missing", "pages#relative_missing")
