class User extends Model
    scope("admins", fn() this.where("doc.role == @r", {"r": "admin"}))
end

def require_admin(req) {
    user = req["user"]
    return {"status": 403, "body": "Forbidden"} unless user.present? && user["role"] == "admin"
    return null
}
