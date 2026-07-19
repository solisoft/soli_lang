# Pages for the browser specs to drive.
#
# Kept deliberately small: each action exists to give one browser behaviour
# something to act on, so a failing spec points at a feature rather than at
# this fixture.
class PagesController extends Controller
    def index(req)
        render("pages/index", {"title": "Home"})
    end

    def about(req)
        render("pages/about", {"title": "About"})
    end

    def form(req)
        render("pages/form", {"title": "Form", "message": ""})
    end

    # Echoes the submitted fields back so a spec can prove the browser really
    # posted them, rather than only that the button was clickable.
    def submit(req)
        let name = params["name"] ?? ""
        let role = params["role"] ?? ""
        let subscribed = params["subscribe"] == "true"

        # Labelled rather than slash-separated: the browser reports visible
        # text, which collapses runs of whitespace, so an empty field between
        # two separators would be indistinguishable from a single space.
        render("pages/form", {
            "title": "Form",
            "message": "Received name=#{name} role=#{role} subscribed=#{subscribed}"
        })
    end

    # Content that only exists after script has run — the difference between a
    # browser test and an HTTP one.
    def dynamic(req)
        render("pages/dynamic", {"title": "Dynamic"})
    end

    # Content that appears on a delay, so waiting is actually exercised rather
    # than accidentally satisfied by a page that was already complete.
    def slow(req)
        render("pages/slow", {"title": "Slow"})
    end

    # Mounts a LiveView component over a real websocket.
    def live(req)
        render("pages/live", {"title": "Live"})
    end

    # Throws in the page, for the page-error assertions.
    def broken(req)
        render("pages/broken", {"title": "Broken"})
    end
end
