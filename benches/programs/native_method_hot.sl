# Hot monomorphic native instance method calls (DateTime accessors).
# Measures the CallMethod path that previously allocated a bound
# NativeFunction wrapper on every call.
let dt = DateTime.from_unix(1704067200);
let sum = 0;
let i = 0;
while (i < 100000) {
    sum = sum + dt.year() + dt.month() + dt.day() + dt.hour() + dt.minute() + dt.second();
    i = i + 1;
}
let result = sum;
