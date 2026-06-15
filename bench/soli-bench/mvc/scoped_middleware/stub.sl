class User extends Model
    # TODO: scope "admins" -> users whose role == "admin"
end

def require_admin(req) {
    # TODO: return null when req's user is an admin, else a 403 response
    return null
}
