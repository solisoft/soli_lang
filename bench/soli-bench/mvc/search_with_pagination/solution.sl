class Article extends Model
end

def search_articles(params) {
    term = params["q"] || ""
    page = params["page"] || 1

    result = Article
        .where("LOWER(doc.title) LIKE @q", {"q": "%" + term.downcase() + "%"})
        .order("title")
        .paginate({"page": page, "per": 2})

    return {
        "status": 200,
        "titles": result["records"].map(fn(article) article.title),
        "total": result["pagination"]["total"],
        "total_pages": result["pagination"]["total_pages"]
    }
}
