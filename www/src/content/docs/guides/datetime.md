---
title: Date and Time
description: Working with dates and times in Soli
---

# Date and Time

Soli provides comprehensive date and time support through native functions and two built-in classes: `DateTime` for moments in time and `Duration` for time spans.

## Quick Start

```rust
// Get current datetime
let now = new DateTime();
print(now.to_string());  // "2025-01-20 15:30:00"

// Parse a date string
let parsed = new DateTime("2025-01-20T15:30:00");
print(parsed.year());     // 2025
print(parsed.month());    // 1
print(parsed.day());      // 20

// Add 5 days
let future = now.add_days(5);
print(future.to_string());

// Calculate duration between dates
let start = new DateTime("2025-01-01");
let end = new DateTime("2025-01-10");
let dur = Duration.between(start, end);
print("Days between: " + str(dur.total_days()));  // 9
```

## DateTime Class

The `DateTime` class represents a specific moment in time.

### Creating DateTime Instances

```rust
// Current local time
let now = new DateTime();

// Current UTC time
let utc = DateTime.utc();

// From Unix timestamp
let from_ts = new DateTime(1737394200);

// From ISO string
let parsed = new DateTime("2025-01-20T15:30:00");
let from_date = new DateTime("2025-01-20");  // Date only, time = 00:00:00
```

### Accessing Components

```rust
let dt = new DateTime();

print(dt.year());      // e.g., 2025
print(dt.month());     // e.g., 1 (1-12)
print(dt.day());       // e.g., 20 (1-31)
print(dt.hour());      // e.g., 15 (0-23)
print(dt.minute());    // e.g., 30 (0-59)
print(dt.second());    // e.g., 0 (0-59)
print(dt.weekday());   // e.g., "monday"
```

### Formatting

```rust
let dt = new DateTime();

// Default format
print(dt.to_string());           // "2025-01-20 15:30:00"

// Custom formatting
print(dt.format("%Y-%m-%d"));           // "2025-01-20"
print(dt.format("%m/%d/%Y"));           // "01/20/2025"
print(dt.format("%H:%M:%S"));           // "15:30:00"
print(dt.format("%A, %B %d, %Y"));     // "Monday, January 20, 2025"
print(dt.format("%Y%m%d"));            // "20250120"

// ISO format
print(dt.to_iso());             // "2025-01-20T15:30:00+00:00"
```

### Date Arithmetic

```rust
let dt = new DateTime();

// Add time
let tomorrow = dt.add_days(1);
let next_week = dt.add_weeks(1);
let next_month = dt.add_months(1);
let next_year = dt.add_years(1);

// Add hours
let later = dt.add_hours(3);

// Add Duration
let dur = Duration.hours(2);
let meeting = dt.add(dur);

// Subtract time
let yesterday = dt.add_days(-1);  // or use sub()
let dur = Duration.days(5);
let past = dt.sub(dur);
```

### Comparison

```rust
let dt1 = new DateTime();
let dt2 = dt1.add_days(1);

if (dt1.is_before(dt2)) {
    print("dt1 is earlier");
}

if (dt2.is_after(dt1)) {
    print("dt2 is later");
}

if (dt1.is_same(dt1.clone())) {
    print("Same moment");
}
```

## Duration Class

The `Duration` class represents a length of time.

### Creating Durations

```rust
// From value and unit
let dur1 = new Duration(5, "days");
let dur2 = new Duration(2, "hours");
let dur3 = new Duration(30, "minutes");

// Using static methods
let dur4 = Duration.days(5);
let dur5 = Duration.hours(2);
let dur6 = Duration.minutes(30);
let dur7 = Duration.seconds(45);
let dur8 = Duration.milliseconds(500);
let dur9 = Duration.weeks(2);  // 2 weeks = 14 days

// From DateTime difference
let start = new DateTime("2025-01-01");
let end = new DateTime("2025-01-10");
let between = Duration.between(start, end);
```

### Duration Properties

```rust
let dur = Duration.days(5);

print(dur.total_days());      // 5.0
print(dur.total_hours());     // 120.0
print(dur.total_minutes());   // 7200.0
print(dur.total_seconds());   // 432000.0
print(dur.total_millis());    // 432000000.0
print(dur.total_weeks());     // ~0.714
print(dur.total_years());     // ~0.0137 (using 365.25 days/year)

// Human-readable string
print(dur.to_string());  // "5 days"
```

### Duration Arithmetic

```rust
let dur1 = Duration.days(5);
let dur2 = Duration.hours(2);

// Addition
let combined = dur1 + dur2;      // 5 days + 2 hours

// Subtraction
let diff = dur1 - Duration.hours(12);  // 5 days - 12 hours

// Multiplication
let doubled = dur1 * 2;          // 10 days

// Division
let halved = dur1 / 2;           // 2.5 days
```

### Using Duration with DateTime

```rust
let meeting = new DateTime("2025-01-20T14:00:00");
let duration = Duration.minutes(90);

let end_time = meeting.add(duration);
print("Meeting ends at: " + end_time.to_string());

// Meeting duration between start and end
let start = new DateTime("2025-01-20T14:00:00");
let end = new DateTime("2025-01-20T15:30:00");
let meeting_length = Duration.between(start, end);
print("Meeting length: " + meeting_length.to_string());  // "1.5 hours"
```

## Format Specifiers

The `format()` method uses format specifiers similar to C's strftime:

| Specifier | Description | Example |
|-----------|-------------|---------|
| `%Y` | Year (4 digits) | 2025 |
| `%y` | Year (2 digits) | 25 |
| `%m` | Month (01-12) | 01 |
| `%d` | Day of month (01-31) | 20 |
| `%H` | Hour (00-23) | 15 |
| `%I` | Hour (01-12) | 03 |
| `%M` | Minute (00-59) | 30 |
| `%S` | Second (00-59) | 00 |
| `%A` | Weekday name | Monday |
| `%a` | Weekday abbreviation | Mon |
| `%B` | Month name | January |
| `%b` | Month abbreviation | Jan |
| `%j` | Day of year (001-366) | 020 |
| `%U` | Week number (Sunday) | 03 |
| `%W` | Week number (Monday) | 03 |
| `%p` | AM/PM | PM |
| `%P` | am/pm | pm |
| `%z` | Timezone offset | +0000 |
| `%Z` | Timezone name | UTC |
| `%s` | Unix timestamp | 1737394200 |

## Common Patterns

### Age Calculation

```rust
fn calculate_age(birthday: DateTime) -> Float {
    let now = new DateTime();
    let age = Duration.between(birthday, now);
    return age.total_years();
}

let birthday = new DateTime("2000-01-01");
let age = calculate_age(birthday);
print("Age: " + str(age) + " years");
```

### Business Days

```rust
fn add_business_days(start: DateTime, days: Int) -> DateTime {
    let result = start.clone();
    let remaining = days;
    while (remaining > 0) {
        result = result.add_days(1);
        if (result.weekday() != "saturday" && result.weekday() != "sunday") {
            remaining = remaining - 1;
        }
    }
    return result;
}

let friday = new DateTime("2025-01-17");  // A Friday
let tuesday = add_business_days(friday, 1);  // Next Tuesday (skipping weekend)
```

### Time Until Event

```rust
fn days_until(event_date: DateTime) -> Float {
    let now = new DateTime();
    let dur = Duration.between(now, event_date);
    return dur.total_days();
}

let conference = new DateTime("2025-03-15");
let days = days_until(conference);
print("Days until conference: " + str(days));
```

### Weekly Schedule

```rust
fn schedule_weekly(meeting: DateTime, duration: Duration, weeks: Int) -> Array {
    let schedule = [];
    let current = meeting.clone();
    for (i in range(0, weeks)) {
        push(schedule, current.to_string());
        current = current.add_weeks(1);
    }
    return schedule;
}

let first = new DateTime("2025-01-06T10:00:00");  // First Monday
let dur = Duration.minutes(60);
let schedule = schedule_weekly(first, dur, 4);
// Returns 4 weekly meeting times
```

## Timezones

Soli uses the system's local timezone by default. For UTC:

```rust
let utc_now = DateTime.utc();
```

For timezone-aware operations, use ISO format with timezone:

```rust
let dt = new DateTime("2025-01-20T15:30:00+05:00");
print("Local equivalent: " + dt.to_string());
```

## Best Practices

1. **Use Unix timestamps for calculations**: Store timestamps as `Int` for precise calculations
2. **Use Duration for arithmetic**: Makes code more readable than adding raw seconds
3. **Handle null from parse**: `__datetime_parse()` returns `null` for invalid formats
4. **Use format() for display**: Keeps display logic separate from data
5. **Consider timezones**: Be aware that comparisons use UTC internally

## See Also

- [Built-in Functions Reference](/docs/reference/builtins) - Complete function reference
- [Classes Guide](/docs/guides/classes) - Class syntax and usage
