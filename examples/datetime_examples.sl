// DateTime and Duration Examples for Soli
// Demonstrates the built-in DateTime and Duration classes

print("=== DateTime Examples ===");

// 1. Get current datetime
let now = DateTime.now();
print("1. Current datetime: " + now.to_string());

// 2. Access date components
print("2. Year: " + str(now.year()) + ", Month: " + str(now.month()) + ", Day: " + str(now.day()));
print("   Hour: " + str(now.hour()) + ", Minute: " + str(now.minute()) + ", Second: " + str(now.second()));
print("   Weekday: " + now.weekday());

// 3. Get Unix timestamp
let unix_ts = now.to_unix();
print("3. Unix timestamp: " + str(unix_ts));

// 4. Custom formats
print("4. Custom formats:");
print("   Full: " + now.format("%A, %B %d, %Y"));
print("   US: " + now.format("%m/%d/%Y"));

// 5. Date arithmetic
print("5. Date arithmetic:");
let tomorrow = now.add_days(1);
print("   Tomorrow: " + tomorrow.to_string());
let next_week = now.add_days(7);
print("   Next week: " + next_week.to_string());

// 6. DateTime comparison
print("6. DateTime comparison:");
if (tomorrow.to_unix() > now.to_unix()) {
    print("   tomorrow is after now: true");
}

// 7. Create from Unix timestamp
let epoch = DateTime.from_unix(0);
print("7. Unix epoch: " + epoch.to_iso());

// 8. Parse from string
let parsed = DateTime.parse("2024-06-15T14:30:00Z");
print("8. Parsed datetime: " + parsed.to_string());

// 9. Get ISO 8601 format
print("9. ISO 8601 format: " + now.to_iso());

print("");
print("=== Duration Examples ===");

// 10. Create durations using factory methods
let dur1 = Duration.of_days(5);
print("10. Duration of 5 days: " + str(dur1.total_days()) + " days");

let dur2 = Duration.of_hours(2);
print("    Duration of 2 hours: " + str(dur2.total_hours()) + " hours");

let dur3 = Duration.of_minutes(30);
print("    Duration of 30 minutes: " + str(dur3.total_minutes()) + " minutes");

let dur4 = Duration.of_seconds(90);
print("    Duration of 90 seconds: " + str(dur4.total_seconds()) + " seconds");

// 11. Duration totals
print("11. Duration totals:");
print("    5 days in hours: " + str(dur1.total_hours()));
print("    2 hours in minutes: " + str(dur2.total_minutes()));

// 12. Create duration between two datetimes
let start = DateTime.now();
let finish = DateTime.now();
let diff = Duration.between(start, finish);
print("12. Duration between now and now: " + str(diff.total_seconds()) + " seconds");

// 13. Using Duration.weeks()
let week_dur = Duration.of_weeks(1);
print("13. Duration of 1 week: " + str(week_dur.total_days()) + " days");

print("");
print("All DateTime and Duration examples completed!");
