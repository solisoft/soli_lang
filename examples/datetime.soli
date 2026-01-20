// DateTime class for date and time manipulation
// Uses the underlying native datetime functions

class DateTime {
    // Internal timestamp stored as Int
    timestamp: Int;

    // Constructor - creates current datetime
    new() {
        self.timestamp = __datetime_now_local();
    }

    // Constructor from Unix timestamp
    new(ts: Int) {
        self.timestamp = ts;
    }

    // Constructor from ISO string
    new(s: String) {
        let parsed = __datetime_parse(s);
        if (parsed == null) {
            print("Error: Invalid datetime format");
            self.timestamp = __datetime_now_local();
        } else {
            self.timestamp = parsed;
        }
    }

    // Get year
    year(): Int {
        let components = __datetime_components(self.timestamp);
        return components["year"];
    }

    // Get month (1-12)
    month(): Int {
        let components = __datetime_components(self.timestamp);
        return components["month"];
    }

    // Get day (1-31)
    day(): Int {
        let components = __datetime_components(self.timestamp);
        return components["day"];
    }

    // Get hour (0-23)
    hour(): Int {
        let components = __datetime_components(self.timestamp);
        return components["hour"];
    }

    // Get minute (0-59)
    minute(): Int {
        let components = __datetime_components(self.timestamp);
        return components["minute"];
    }

    // Get second (0-59)
    second(): Int {
        let components = __datetime_components(self.timestamp);
        return components["second"];
    }

    // Get weekday name
    weekday(): String {
        return __datetime_weekday(self.timestamp);
    }

    // Get Unix timestamp
    to_unix(): Int {
        return self.timestamp;
    }

    // Get ISO 8601 formatted string
    to_iso(): String {
        return __datetime_to_iso(self.timestamp);
    }

    // Format datetime with custom format string
    // Format specifiers: %Y (year), %m (month), %d (day), %H (hour), %M (minute), %S (second), %A (weekday)
    format(fmt: String): String {
        return __datetime_format(self.timestamp, fmt);
    }

    // Get formatted string (locale-friendly default format)
    to_string(): String {
        return self.format("%Y-%m-%d %H:%M:%S");
    }

    // Add days to datetime
    add_days(n: Int): DateTime {
        let new_ts = __datetime_add_days(self.timestamp);
        return new DateTime(new_ts);
    }

    // Add hours to datetime
    add_hours(n: Int): DateTime {
        let new_ts = __datetime_add_hours(self.timestamp);
        return new DateTime(new_ts);
    }

    // Add weeks to datetime
    add_weeks(n: Int): DateTime {
        let new_ts = __datetime_add_weeks(self.timestamp);
        return new DateTime(new_ts);
    }

    // Add months to datetime
    add_months(n: Int): DateTime {
        let new_ts = __datetime_add_months(self.timestamp);
        return new DateTime(new_ts);
    }

    // Add years to datetime
    add_years(n: Int): DateTime {
        let new_ts = __datetime_add_years(self.timestamp);
        return new DateTime(new_ts);
    }

    // Add duration to datetime
    add(dur: Duration): DateTime {
        let seconds = dur.total_seconds();
        let new_ts = __datetime_add(self.timestamp, seconds);
        return new DateTime(new_ts);
    }

    // Subtract duration from datetime
    sub(dur: Duration): DateTime {
        let seconds = dur.total_seconds();
        let new_ts = __datetime_sub(self.timestamp, seconds);
        return new DateTime(new_ts);
    }

    // Check if this datetime is before another
    is_before(other: DateTime): Bool {
        return __datetime_is_before(self.timestamp, other.timestamp);
    }

    // Check if this datetime is after another
    is_after(other: DateTime): Bool {
        return __datetime_is_after(self.timestamp, other.timestamp);
    }

    // Check if this datetime is the same as another
    is_same(other: DateTime): Bool {
        return __datetime_is_same(self.timestamp, other.timestamp);
    }

    // Clone this datetime
    clone(): DateTime {
        return new DateTime(self.timestamp);
    }
}

// Current UTC datetime
fn DateTime.utc(): DateTime {
    let dt = new DateTime();
    dt.timestamp = __datetime_now_utc();
    return dt;
}

// Parse datetime from string
fn DateTime.parse(s: String): DateTime {
    return new DateTime(s);
}
