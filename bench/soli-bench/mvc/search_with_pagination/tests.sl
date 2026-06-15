describe("search_with_pagination", fn() {
    before_each(fn() {
        Article.all.each(fn(article) article.delete())
        Article.create({"title": "Alpha Guide"})
        Article.create({"title": "Beta Guide"})
        Article.create({"title": "Gamma Guide"})
        Article.create({"title": "Random Note"})
    })

    test("paginates matches, ordered and capped at 2 per page", fn() {
        res = search_articles({"q": "guide", "page": 1})
        assert_eq(res["status"], 200)
        assert_eq(res["titles"], ["Alpha Guide", "Beta Guide"])
        assert_eq(res["total"], 3)
        assert_eq(res["total_pages"], 2)
    })

    test("returns the second page", fn() {
        res = search_articles({"q": "guide", "page": 2})
        assert_eq(res["titles"], ["Gamma Guide"])
    })

    test("empty query matches everything", fn() {
        res = search_articles({})
        assert_eq(res["total"], 4)
    })
})
