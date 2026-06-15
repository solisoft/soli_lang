class Post extends Model
    validates(:title, presence: true)
end

def create_post(params) {
    post = Post.create(params)
    if post._errors {
        return {"status": 422, "errors": post.errors}
    }
    return {"status": 201, "key": post._key, "title": post.title}
}

def show_post(key) {
    post = Post.find(key)
    return {"status": 200, "title": post.title}
}

def update_post(key, params) {
    post = Post.find(key)
    post.update(params)
    return {"status": 200, "title": post.title}
}

def delete_post(key) {
    Post.find(key).delete
    return {"status": 204}
}

def index_posts {
    return {"status": 200, "count": Post.all.length}
}
