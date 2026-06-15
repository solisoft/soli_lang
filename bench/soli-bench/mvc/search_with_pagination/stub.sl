class Article extends Model
end

def search_articles(params) {
    # TODO: filter by params["q"] (case-insensitive title substring),
    #       order titles A->Z, paginate with per-page size 2, and return
    #       {"status", "titles", "total", "total_pages"}.
    return {"status": 500}
}
