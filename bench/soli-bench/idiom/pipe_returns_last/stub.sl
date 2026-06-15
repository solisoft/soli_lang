def admin_emails(users) {
    let result = [];
    for user in users {
        if user["active"] {
            if user["role"] == "admin" {
                if user["email"].present? {
                    if user["email"].contains("@") {
                        result.push(user["email"]);
                    }
                }
            }
        }
    }
    return result.sort;
}
