def display_name(user) {
    if user == null {
        return "Anonymous";
    }
    return user["name"];
}

def is_registered(user) {
    return user != null;
}
