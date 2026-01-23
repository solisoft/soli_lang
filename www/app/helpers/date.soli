// Date/time formatting helpers

fn format_date(timestamp: Int, format: String) -> String {
    return datetime_format(timestamp, format);
}

fn format_datetime(timestamp: Int) -> String {
    return datetime_format(timestamp, "%Y-%m-%d %H:%M:%S");
}

fn format_relative(timestamp: Int) -> String {
    return time_ago(timestamp);
}

fn is_today(timestamp: Int) -> Bool {
    let now = datetime_now();
    let day_start = datetime_add_days(timestamp, 0);
    let day_end = datetime_add_days(timestamp, 86400);
    return now >= day_start && now < day_end;
}
