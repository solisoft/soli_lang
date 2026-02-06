// Home controller - handles routes at /

class HomeController extends Controller {
    // GET /up
    fn up(req) {
        return render_text("UP");
    }

    // GET /
    fn index(req) {
        return render("home/index", {
            "title": "Welcome",
            "message": "The Modern MVC Framework for Soli"
        });
    }

    // GET /health
    fn health(req: Any) -> Any {
        return render_json({
            "status": "ok"
        });
    }

    // GET /docs - redirect to documentation
    fn docs_redirect(req: Any) -> Any {
        return {
            "status": 302,
            "headers": {"Location": "/docs.html"},
            "body": ""
        };
    }

    // GET /files/*filepath - Splat route demo
    fn files_demo(req) {
        return render_json({
            "route": "files_demo",
            "params": req["params"]
        });
    }

    // GET /api/*version/users/*id - Multi-splat route demo
    fn api_demo(req) {
        return render_json({
            "route": "api_demo",
            "params": req["params"]
        });
    }

    // GET /*catchall - Catch-all route demo
    fn catchall_demo(req) {
        return render_json({
            "route": "catchall_demo",
            "params": req["params"]
        });
    }
}
