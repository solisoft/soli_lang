// Home Controller
def index(req: Any) -> Any {
    return render("home/index.html", {
        "title": "Welcome",
        "message": "Welcome to your Soli app!"
    });
}

def health(req: Any) -> Any {
    return {
        "status": "ok",
        "timestamp": clock()
    };
}
