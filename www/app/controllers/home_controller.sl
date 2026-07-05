# Home controller - handles routes at /

class HomeController extends Controller
    # GET /up
    def up
        render_text("UP")
    end

    # GET /
    def index
        render("home/index", {
            "title": "Welcome",
            "message": "The Modern MVC Framework for Soli"
        })
    end

    # GET /health
    def health
        render_json({
            "status": "ok"
        })
    end

    # GET /docs - redirect to documentation
    def docs_redirect
        {
            "status": 302,
            "headers": {"Location": "/docs.html"},
            "body": ""
        }
    end

    # GET /files/*filepath - Splat route demo
    def files_demo
        render_json({
            "route": "files_demo",
            "params": req["params"]
        })
    end

    # GET /api/*version/users/*id - Multi-splat route demo
    def api_demo
        render_json({
            "route": "api_demo",
            "params": req["params"]
        })
    end

    # GET /*catchall - Catch-all route demo
    def catchall_demo
        render_json({
            "route": "catchall_demo",
            "params": req["params"]
        })
    end
end
