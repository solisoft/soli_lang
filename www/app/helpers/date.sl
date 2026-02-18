// Date/time formatting helpers

fn format_date(timestamp: Int, format: String) -> String
    datetime_format(timestamp, format)
end

fn format_datetime(timestamp: Int) -> String
    datetime_format(timestamp, "%Y-%m-%d %H:%M:%S")
end

fn format_relative(timestamp: Int) -> String
    time_ago(timestamp)
end

fn is_today(timestamp: Int) -> Bool
    let now = DateTime.now();
    let day_start = datetime_add_days(timestamp, 0);
    let day_end = datetime_add_days(timestamp, 86400);
    now >= day_start && now < day_end
end
