fn public_path(req) {
    render("test/public_path", {
        "title": "Public Path Test"
    })
}

fn h_test(req) {
    render("test/h_test", {
        "title": "h() Function Test"
    })
}
