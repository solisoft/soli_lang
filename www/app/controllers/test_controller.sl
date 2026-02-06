fn public_path(req: Any) -> Any {
    render("test/public_path", {
        "title": "Public Path Test"
    })
}

fn h_test(req: Any) -> Any {
    render("test/h_test", {
        "title": "h() Function Test"
    })
}
