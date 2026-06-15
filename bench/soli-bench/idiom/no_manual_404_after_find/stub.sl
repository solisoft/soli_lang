def show(id) {
    post = Post.find(id)
    if post.nil? {
        return {"status": 404, "title": "Not Found"}
    }
    return {"status": 200, "title": post["title"]}
}
