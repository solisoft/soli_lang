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

    # GET /ai
    def ai
        evals = {
            "generated_at": null,
            "harness": "soli-evals/0.2",
            "runs_per_model": 3,
            "models": [],
            "tasks": [],
            "note": "No paid run committed yet."
        }
        path = "data/ai_evals.json"
        path = "www/data/ai_evals.json" unless File.exists(path)
        if File.exists(path)
            evals = json_parse(File.read(path)) rescue evals
        end
        render("home/ai", {
            "title": "Soli is built for AI",
            "evals": evals
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
