def admin_emails(users) {
    return users
        .filter(fn(user) user["active"] && user["role"] == "admin")
        .filter(fn(user) user["email"].present? && user["email"].contains("@"))
        .map(fn(user) user["email"])
        .sort;
}
