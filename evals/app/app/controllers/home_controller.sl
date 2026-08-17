class HomeController extends Controller
    def index
        render("home/index", {
            "title": "Shop"
        })
    end

    def health
        render_text("UP")
    end
end
