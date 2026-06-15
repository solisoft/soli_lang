def is_valid_status(status) {
    return ["up", "late", "overdue"].includes?(status);
}

def is_privileged(role) {
    return !["guest", "banned", "pending"].includes?(role);
}
