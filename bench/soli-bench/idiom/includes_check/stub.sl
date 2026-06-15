def is_valid_status(status) {
    return status == "up" || status == "late" || status == "overdue";
}

def is_privileged(role) {
    return role != "guest" && role != "banned" && role != "pending";
}
