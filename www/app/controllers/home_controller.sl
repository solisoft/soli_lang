# Home controller - handles routes at /

class HomeController extends Controller
    # GET /up
    fn up
        render_text("UP")
    end

    # GET /
    fn index
        render("home/index", {
            "title": "Welcome",
            "message": "The Modern MVC Framework for Soli"
        })
    end

    # GET /health
    fn health
        render_json({
            "status": "ok"
        })
    end

    # GET /docs - redirect to documentation
    fn docs_redirect
        {
            "status": 302,
            "headers": {"Location": "/docs.html"},
            "body": ""
        }
    end

    # GET /files/*filepath - Splat route demo
    fn files_demo
        render_json({
            "route": "files_demo",
            "params": req["params"]
        })
    end

    # GET /api/*version/users/*id - Multi-splat route demo
    fn api_demo
        render_json({
            "route": "api_demo",
            "params": req["params"]
        })
    end

    # GET /*catchall - Catch-all route demo
    fn catchall_demo
        render_json({
            "route": "catchall_demo",
            "params": req["params"]
        })
    end
end
