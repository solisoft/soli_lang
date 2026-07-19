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

# LiveView: the socket endpoint plus the page that mounts it.
router_live("counter", "live#counter")
get("/live", "pages#live")
