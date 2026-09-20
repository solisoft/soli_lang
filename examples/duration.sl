# Duration — time spans, from the built-in class.
#
# This file used to be a 152-line userland `Duration` class: a constructor
# taking a value and a unit, six `static` convenience constructors, seven
# `total_*` accessors and four operator overloads. It never ran. It was written
# against `static` methods, `self.`, `value as Float` and `new Duration(...)` —
# four constructs Soli does not have — and nothing executed `examples/`, so it
# sat there being wrong for as long as it existed.
#
# It was also reimplementing something Soli ships. `Duration` is a built-in,
# with the same surface that file was building by hand, so the example now
# shows the thing itself rather than a copy of it.
#
#   soli examples/duration.sl

# Constructors. Each takes a count and answers a span.
let quarter_hour = Duration.minutes(15)
let working_day  = Duration.hours(8)
let sprint       = Duration.days(14)

print("a quarter hour in seconds: #{quarter_hour.total_seconds()}")
print("a working day in minutes:  #{working_day.total_minutes()}")
print("a sprint in hours:         #{sprint.total_hours()}")

# The accessors convert rather than truncate, so they answer a float and the
# unit you ask for is the unit you get.
print("a sprint in days:          #{sprint.total_days()}")

# `humanize` is for showing someone, not for arithmetic.
print(Duration.seconds(3661).humanize())   # 1 hour 1 minute
print(Duration.seconds(45).humanize())
print(Duration.minutes(90).humanize())

# `between` measures two instants. Note that `DateTime.now()` carries a
# subsecond, so this is a real measurement and not a whole number of seconds.
let started = DateTime.now()
let total   = 0
for i in 1..200000 {
    total = total + i
}
let elapsed = Duration.between(started, DateTime.now())

print("summed to #{total} in #{elapsed.total_seconds()}s")

# A week is seven days; there is no separate constructor for it, because
# multiplying the count is clearer than another name to remember.
let fortnight = Duration.days(7 * 2)
print("a fortnight in days:       #{fortnight.total_days()}")
