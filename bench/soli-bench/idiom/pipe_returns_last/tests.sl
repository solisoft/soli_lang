let sample_users = [
    {"active": true,  "role": "admin", "email": "zoe@x.com"},
    {"active": true,  "role": "admin", "email": "amy@x.com"},
    {"active": false, "role": "admin", "email": "ben@x.com"},
    {"active": true,  "role": "user",  "email": "cara@x.com"},
    {"active": true,  "role": "admin", "email": "no-at-sign"}
];

describe("pipe_returns_last", fn() {
    test("returns sorted active admin emails with @", fn() {
        assert_eq(admin_emails(sample_users), ["amy@x.com", "zoe@x.com"]);
    });

    test("empty input", fn() {
        assert_eq(admin_emails([]), []);
    });
});
