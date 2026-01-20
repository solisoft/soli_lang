// DateTime and Duration Examples for Soli
// Simple self-contained example

// Simple DateTime wrapper class
class DateTime {
    timestamp: Int;

    new() {
        this.timestamp = __datetime_now_local();
    }

    fn year() -> Int {
        let components = __datetime_components(this.timestamp);
        return components["year"];
    }

    fn month() -> Int {
        let components = __datetime_components(this.timestamp);
        return components["month"];
    }

    fn day() -> Int {
        let components = __datetime_components(this.timestamp);
        return components["day"];
    }

    fn hour() -> Int {
        let components = __datetime_components(this.timestamp);
        return components["hour"];
    }

    fn minute() -> Int {
        let components = __datetime_components(this.timestamp);
        return components["minute"];
    }

    fn second() -> Int {
        let components = __datetime_components(this.timestamp);
        return components["second"];
    }

    fn weekday() -> String {
        return __datetime_weekday(this.timestamp);
    }

    fn to_unix() -> Int {
        return this.timestamp;
    }

    fn to_iso() -> String {
        return __datetime_to_iso(this.timestamp);
    }

    fn format(fmt: String) -> String {
        return __datetime_format(this.timestamp, fmt);
    }

    fn to_string() -> String {
        return this.format("%Y-%m-%d %H:%M:%S");
    }

    fn add_days(n: Int) -> DateTime {
        let new_ts = __datetime_add_days(this.timestamp);
        let result = new DateTime();
        result.timestamp = new_ts;
        return result;
    }

    fn add_hours(n: Int) -> DateTime {
        let new_ts = __datetime_add_hours(this.timestamp);
        let result = new DateTime();
        result.timestamp = new_ts;
        return result;
    }

    fn add_weeks(n: Int) -> DateTime {
        let new_ts = __datetime_add_weeks(this.timestamp);
        let result = new DateTime();
        result.timestamp = new_ts;
        return result;
    }

    fn add_months(n: Int) -> DateTime {
        let new_ts = __datetime_add_months(this.timestamp);
        let result = new DateTime();
        result.timestamp = new_ts;
        return result;
    }

    fn add_years(n: Int) -> DateTime {
        let new_ts = __datetime_add_years(this.timestamp);
        let result = new DateTime();
        result.timestamp = new_ts;
        return result;
    }

    fn add(dur: Duration) -> DateTime {
        let seconds = dur.total_seconds();
        let new_ts = __datetime_add(this.timestamp, seconds);
        let result = new DateTime();
        result.timestamp = new_ts;
        return result;
    }

    fn sub(dur: Duration) -> DateTime {
        let seconds = dur.total_seconds();
        let new_ts = __datetime_sub(this.timestamp, seconds);
        let result = new DateTime();
        result.timestamp = new_ts;
        return result;
    }

    fn is_before(other: DateTime) -> Bool {
        return __datetime_is_before(this.timestamp, other.timestamp);
    }

    fn is_after(other: DateTime) -> Bool {
        return __datetime_is_after(this.timestamp, other.timestamp);
    }

    fn is_same(other: DateTime) -> Bool {
        return __datetime_is_same(this.timestamp, other.timestamp);
    }
}

class Duration {
    total_secs: Float;

    new(value: Int, unit: String) {
        let multiplier = 1.0;
        if (unit == "days") {
            multiplier = 86400.0;
        } else if (unit == "hours") {
            multiplier = 3600.0;
        } else if (unit == "minutes") {
            multiplier = 60.0;
        } else if (unit == "seconds") {
            multiplier = 1.0;
        } else if (unit == "milliseconds") {
            multiplier = 0.001;
        } else {
            print("Error: Unknown unit: " + unit);
            this.total_secs = 0.0;
            return;
        }
        this.total_secs = float(value) * multiplier;
    }

    fn total_days() -> Float {
        return this.total_secs / 86400.0;
    }

    fn total_hours() -> Float {
        return this.total_secs / 3600.0;
    }

    fn total_minutes() -> Float {
        return this.total_secs / 60.0;
    }

    fn total_seconds() -> Float {
        return this.total_secs;
    }

    fn to_string() -> String {
        let days = this.total_days();
        if (days >= 1.0) {
            return str(days) + " days";
        }
        let hours = this.total_hours();
        if (hours >= 1.0) {
            return str(hours) + " hours";
        }
        let minutes = this.total_minutes();
        if (minutes >= 1.0) {
            return str(minutes) + " minutes";
        }
        return str(this.total_secs) + " seconds";
    }
}

// ============================================
// EXAMPLES START HERE
// ============================================

print("=== DateTime Examples ===");

let now = new DateTime();
print("1. Current datetime: " + now.to_string());

print("2. Year: " + str(now.year()) + ", Month: " + str(now.month()) + ", Day: " + str(now.day()));
print("   Hour: " + str(now.hour()) + ", Minute: " + str(now.minute()) + ", Second: " + str(now.second()));
print("   Weekday: " + now.weekday());

let unix_ts = now.to_unix();
print("3. Unix timestamp: " + str(unix_ts));

print("4. Custom formats:");
print("   Full: " + now.format("%A, %B %d, %Y"));
print("   US: " + now.format("%m/%d/%Y"));

print("5. Date arithmetic:");
let tomorrow = now.add_days(1);
print("   Tomorrow: " + tomorrow.to_string());
let next_week = now.add_weeks(1);
print("   Next week: " + next_week.to_string());

print("6. DateTime comparison:");
if (now.is_before(tomorrow)) {
    print("   now is before tomorrow: true");
}
if (tomorrow.is_after(now)) {
    print("   tomorrow is after now: true");
}

print("");
print("=== Duration Examples ===");

let dur1 = new Duration(5, "days");
print("7. Duration of 5 days: " + dur1.to_string());

let dur2 = new Duration(2, "hours");
print("   Duration of 2 hours: " + dur2.to_string());

let dur3 = new Duration(30, "minutes");
print("   Duration of 30 minutes: " + dur3.to_string());

print("8. Duration totals:");
print("   5 days in hours: " + str(dur1.total_hours()));
print("   2 hours in minutes: " + str(dur2.total_minutes()));

print("9. DateTime + Duration:");
let later = now.add(dur1);
print("   Now + 5 days: " + later.to_string());

print("10. Duration weeks and totals:");
let week_dur = new Duration(1, "weeks");
print("   1 week in days: " + str(week_dur.total_days()));
print("   1 week in hours: " + str(week_dur.total_hours()));

print("11. Total years example:");
let year_dur = new Duration(365, "days");
print("   365 days in years: " + str(year_dur.total_days() / 365.25));

print("");
print("All DateTime examples completed!");
