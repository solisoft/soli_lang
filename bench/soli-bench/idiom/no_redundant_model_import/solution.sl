def post_titles {
    return Post.all.map(fn(post) post["title"]);
}
