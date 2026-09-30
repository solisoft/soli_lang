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

    # `render("relative")` from `pages#relative` resolves to pages/relative;
    # `render("standalone")` has no pages/standalone, so it falls back to the top-level view.
    def relative(req)
        render("relative", {"title": "Relative"})
    end

    def relative_fallback(req)
        render("standalone", {"title": "Standalone"})
    end

    # Neither pages/nowhere nor a top-level nowhere exists. The test server is not
    # in --dev, so show the error message rather than the generic 500 page.
    def relative_missing(req)
        try
            return render("nowhere", {"title": "Missing"})
        catch error
            return {"status": 500, "headers": {"Content-Type": "text/plain"}, "body": "#{error}"}
        end
    end

    # A LiveView file input with pause/resume, for the upload specs.
    def uploads(req)
        render("pages/uploads", {"title": "Uploads"})
    end

    # Two pages on a layout that opts into morphing instead of body swapping.
    def morph_a(req)
        render("pages/morph_a", {"title": "Morph A", "layout": "layouts/morph"})
    end

    def morph_b(req)
        render("pages/morph_b", {"title": "Morph B", "layout": "layouts/morph"})
    end

    # Throws in the page, for the page-error assertions.
    def broken(req)
        render("pages/broken", {"title": "Broken"})
    end
end
