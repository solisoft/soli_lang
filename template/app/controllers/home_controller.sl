// Home Controller
fn index(req: Any) -> Any {
    return render("home/index.html", {
        "title": "Welcome",
        "message": "Welcome to your Soli app!"
    });
}

fn health(req: Any) -> Any {
    return {
        "status": "ok",
        "timestamp": clock()
    };
}
