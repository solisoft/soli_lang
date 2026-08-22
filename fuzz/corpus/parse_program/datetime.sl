// DateTime example demonstrating built-in DateTime and Duration classes

print("=== DateTime Built-in Class Demo ===");

// Create current datetime
let now = DateTime.now();
print("Current time: " + now.to_string());

// Access components
print("Year: " + str(now.year()));
print("Month: " + str(now.month()));
print("Day: " + str(now.day()));
print("Hour: " + str(now.hour()));
print("Minute: " + str(now.minute()));
print("Second: " + str(now.second()));
print("Weekday: " + now.weekday());

// Create from timestamp
let epoch = DateTime.from_unix(0);
print("\nUnix epoch: " + epoch.to_iso());

// Create from string
let dt = DateTime.parse("2024-01-01T00:00:00Z");
print("Parsed: " + dt.to_string());

// Date arithmetic
let tomorrow = now.add_days(1);
let next_hour = now.add_hours(1);
print("\nTomorrow: " + tomorrow.to_string());
print("Next hour: " + next_hour.to_string());

// Formatting
print("\nFormatted: " + now.format("%Y-%m-%d %H:%M:%S"));
print("ISO format: " + now.to_iso());

print("\n=== Duration Built-in Class Demo ===");

// Create durations
let dur1 = Duration.of_days(3);
let dur2 = Duration.of_hours(5);
let dur3 = Duration.of_minutes(30);

print("3 days = " + str(dur1.total_hours()) + " hours");
print("5 hours = " + str(dur2.total_minutes()) + " minutes");
print("30 minutes = " + str(dur3.total_seconds()) + " seconds");

print("\nDemo completed!");
