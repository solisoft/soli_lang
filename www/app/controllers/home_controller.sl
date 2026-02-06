// Home controller - handles routes at /

class HomeController extends Controller {
    // GET /up
    fn up(req) {
        render_text("UP")
    }

    // GET /
    fn index(req) {
        render("home/index", {
            "title": "Welcome",
            "message": "The Modern MVC Framework for Soli"
        })
    }

    // GET /health
    fn health(req) {
        render_json({
            "status": "ok"
        })
    }

    // GET /docs - redirect to documentation
    fn docs_redirect(req) {
        {
            "status": 302,
            "headers": {"Location": "/docs.html"},
            "body": ""
        }
    }

    // GET /files/*filepath - Splat route demo
    fn files_demo(req) {
        render_json({
            "route": "files_demo",
            "params": req["params"]
        })
    }

    // GET /api/*version/users/*id - Multi-splat route demo
    fn api_demo(req) {
        render_json({
            "route": "api_demo",
            "params": req["params"]
        })
    }

    // GET /*catchall - Catch-all route demo
    fn catchall_demo(req) {
        render_json({
            "route": "catchall_demo",
            "params": req["params"]
        })
    }
}
