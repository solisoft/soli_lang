def display_name(user) {
    if user.nil? {
        return "Anonymous";
    }
    return user["name"];
}

def is_registered(user) {
    return user.present?;
}
