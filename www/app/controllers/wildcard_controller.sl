// Wildcard controller - demonstrates dynamic action resolution
//
// Routes: get("/wildcard/*", "wildcard#*")
// /wildcard/demo -> wildcard#demo
// /wildcard/example -> wildcard#example

class WildcardController extends Controller
    fn demo(req)
        render_text("Wildcard demo action!")
    end

    fn example(req)
        let path = req["params"]["path"]
        render_text("Wildcard example! Path: " + path)
    end
end
