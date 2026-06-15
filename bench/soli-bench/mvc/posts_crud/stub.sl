class Post extends Model
    # TODO: validate presence of title
end

def create_post(params) {
    # TODO: create a Post; return 201 with key + title, or 422 with errors
    return {"status": 500}
}

def show_post(key) {
    # TODO: return 200 with the post's title
    return {"status": 500}
}

def update_post(key, params) {
    # TODO: apply params; return 200 with the new title
    return {"status": 500}
}

def delete_post(key) {
    # TODO: delete the post; return 204
    return {"status": 500}
}

def index_posts {
    # TODO: return 200 with the post count
    return {"status": 500}
}
