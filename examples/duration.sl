// Duration class for time span manipulation
// Internal representation is total seconds as Float

class Duration {
    // Internal total seconds
    total_secs: Float;

    // Constructor - creates duration from value and unit
    // Units: "days", "hours", "minutes", "seconds", "milliseconds"
    new(value: Int, unit: String) {
        let multiplier = 1.0;
        if (unit == "days") {
            multiplier = 86400.0;
        } elsif (unit == "hours") {
            multiplier = 3600.0;
        } elsif (unit == "minutes") {
            multiplier = 60.0;
        } elsif (unit == "seconds") {
            multiplier = 1.0;
        } elsif (unit == "milliseconds") {
            multiplier = 0.001;
        } else {
            print("Error: Unknown duration unit: " + unit);
            print("Valid units: days, hours, minutes, seconds, milliseconds");
            self.total_secs = 0.0;
            return;
        }
        self.total_secs = value as Float * multiplier;
    }

    // Convenience constructor for days
    static days(n: Int): Duration {
        return new Duration(n, "days");
    }

    // Convenience constructor for hours
    static hours(n: Int): Duration {
        return new Duration(n, "hours");
    }

    // Convenience constructor for minutes
    static minutes(n: Int): Duration {
        return new Duration(n, "minutes");
    }

    // Convenience constructor for seconds
    static seconds(n: Int): Duration {
        return new Duration(n, "seconds");
    }

    // Convenience constructor for milliseconds
    static milliseconds(n: Int): Duration {
        return new Duration(n, "milliseconds");
    }

    // Convenience constructor for weeks
    static weeks(n: Int): Duration {
        return new Duration(n * 7, "days");
    }

    // Create duration between two DateTimes
    static between(start: DateTime, end: DateTime): Duration {
        let diff = __datetime_diff(start.to_unix(), end.to_unix());
        let dur = new Duration(0, "seconds");
        dur.total_secs = diff as Float;
        return dur;
    }

    // Get total days
    total_days(): Float {
        return self.total_secs / 86400.0;
    }

    // Get total hours
    total_hours(): Float {
        return self.total_secs / 3600.0;
    }

    // Get total minutes
    total_minutes(): Float {
        return self.total_secs / 60.0;
    }

    // Get total seconds
    total_seconds(): Float {
        return self.total_secs;
    }

    // Get total milliseconds
    total_millis(): Float {
        return self.total_secs * 1000.0;
    }

    // Get total years (approximate, using 365.25 days per year)
    total_years(): Float {
        return self.total_days() / 365.25;
    }

    // Get total weeks
    total_weeks(): Float {
        return self.total_days() / 7.0;
    }

    // Clone this duration
    clone(): Duration {
        let dur = new Duration(0, "seconds");
        dur.total_secs = self.total_secs;
        return dur;
    }

    // Convert to string
    to_string(): String {
        let days = self.total_days();
        if (days >= 1.0) {
            return str(days) + " days";
        }
        let hours = self.total_hours();
        if (hours >= 1.0) {
            return str(hours) + " hours";
        }
        let minutes = self.total_minutes();
        if (minutes >= 1.0) {
            return str(minutes) + " minutes";
        }
        return str(self.total_secs) + " seconds";
    }
}

// Operator: Duration + Duration
fn Duration.+(other: Duration): Duration {
    let result = new Duration(0, "seconds");
    result.total_secs = self.total_secs + other.total_secs;
    return result;
}

// Operator: Duration - Duration
fn Duration.-(other: Duration): Duration {
    let result = new Duration(0, "seconds");
    result.total_secs = self.total_secs - other.total_secs;
    return result;
}

// Operator: Duration * Int
fn Duration.*(n: Int): Duration {
    let result = new Duration(0, "seconds");
    result.total_secs = self.total_secs * (n as Float);
    return result;
}

// Operator: Duration / Int
fn Duration./(n: Int): Duration {
    let result = new Duration(0, "seconds");
    result.total_secs = self.total_secs / (n as Float);
    return result;
}
