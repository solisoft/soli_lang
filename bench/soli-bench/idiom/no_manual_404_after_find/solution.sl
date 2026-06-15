def show(id) {
    post = Post.find(id)
    return {"status": 200, "title": post["title"]}
}
