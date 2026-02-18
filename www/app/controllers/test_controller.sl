fn public_path(req)
    render("test/public_path", {
        "title": "Public Path Test"
    })
end

fn h_test(req)
    render("test/h_test", {
        "title": "h() Function Test"
    })
end
