def public_path
    render("test/public_path", {
        "title": "Public Path Test"
    })
end

def h_test
    render("test/h_test", {
        "title": "h() Function Test"
    })
end
