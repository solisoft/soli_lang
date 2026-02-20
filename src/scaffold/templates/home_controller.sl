# Home controller - handles the root routes

class HomeController extends Controller
    # GET /
    def index(req)
        return render("home/index", {
            "title": "Welcome"
        });
    end

    # GET /health
    def health(req)
        return {
            "status": 200,
            "headers": {"Content-Type": "application/json"},
            "body": "{\"status\":\"ok\"}"
        };
    end
end
