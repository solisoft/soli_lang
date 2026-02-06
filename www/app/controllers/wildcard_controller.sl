// Wildcard controller - demonstrates dynamic action resolution
//
// Routes: get("/wildcard/*", "wildcard#*")
// /wildcard/demo → wildcard#demo
// /wildcard/example → wildcard#example

class WildcardController extends Controller {
    fn demo(req: Any) -> Any {
        return render_text("Wildcard demo action!");
    }

    fn example(req: Any) -> Any {
        let path = req["params"]["path"];
        return render_text("Wildcard example! Path: " + path);
    }
}
