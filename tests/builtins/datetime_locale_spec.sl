// ============================================================================
// DateTime Locale Format Test Suite
// ============================================================================
// Integration tests for DateTime.format() with locale parameter
// ============================================================================

// ============================================================================
// Default behavior (no locale / "en")
// ============================================================================
describe("DateTime.format() default (English)", fn() {
    test("format without locale returns English day name", fn() {
        // 2024-01-01 is a Monday (UTC)
        let dt = DateTime.parse("2024-01-01T00:00:00Z");
        let result = dt.format("%A");
        assert_eq(result, "Monday");
    });

    test("format without locale returns English month name", fn() {
        let dt = DateTime.parse("2024-02-15T10:30:00Z");
        let result = dt.format("%B");
        assert_eq(result, "February");
    });

    test("format with 'en' locale returns English day name", fn() {
        let dt = DateTime.parse("2024-01-01T00:00:00Z");
        let result = dt.format("%A", "en");
        assert_eq(result, "Monday");
    });

    test("format with 'en' locale returns English month name", fn() {
        let dt = DateTime.parse("2024-02-15T10:30:00Z");
        let result = dt.format("%B", "en");
        assert_eq(result, "February");
    });

    test("format with 'en' matches format without locale", fn() {
        let dt = DateTime.parse("2024-06-15T10:30:00Z");
        let without = dt.format("%A %d %B %Y");
        let with_en = dt.format("%A %d %B %Y", "en");
        assert_eq(without, with_en);
    });
});

// ============================================================================
// French locale
// ============================================================================
describe("DateTime.format() with French locale", fn() {
    test("full month names", fn() {
        let months = [
            ["2024-01-15T00:00:00Z", "janvier"],
            ["2024-02-15T00:00:00Z", "février"],
            ["2024-03-15T00:00:00Z", "mars"],
            ["2024-04-15T00:00:00Z", "avril"],
            ["2024-05-15T00:00:00Z", "mai"],
            ["2024-06-15T00:00:00Z", "juin"],
            ["2024-07-15T00:00:00Z", "juillet"],
            ["2024-08-15T00:00:00Z", "août"],
            ["2024-09-15T00:00:00Z", "septembre"],
            ["2024-10-15T00:00:00Z", "octobre"],
            ["2024-11-15T00:00:00Z", "novembre"],
            ["2024-12-15T00:00:00Z", "décembre"]
        ];
        months.each(fn(pair) {
            let dt = DateTime.parse(pair[0]);
            let result = dt.format("%B", "fr");
            assert_eq(result, pair[1]);
        });
    });

    test("full day names", fn() {
        // 2024-01-01=Mon, 02=Tue, 03=Wed, 04=Thu, 05=Fri, 06=Sat, 07=Sun
        let days = [
            ["2024-01-01T00:00:00Z", "lundi"],
            ["2024-01-02T00:00:00Z", "mardi"],
            ["2024-01-03T00:00:00Z", "mercredi"],
            ["2024-01-04T00:00:00Z", "jeudi"],
            ["2024-01-05T00:00:00Z", "vendredi"],
            ["2024-01-06T00:00:00Z", "samedi"],
            ["2024-01-07T00:00:00Z", "dimanche"]
        ];
        days.each(fn(pair) {
            let dt = DateTime.parse(pair[0]);
            let result = dt.format("%A", "fr");
            assert_eq(result, pair[1]);
        });
    });

    test("abbreviated month name (%b)", fn() {
        let dt = DateTime.parse("2024-02-15T10:30:00Z");
        let result = dt.format("%d %b %Y", "fr");
        assert_contains(result, "févr.");
    });

    test("abbreviated day name (%a)", fn() {
        // 2024-03-06 is Wednesday
        let dt = DateTime.parse("2024-03-06T14:30:00Z");
        let result = dt.format("%a %d %B", "fr");
        assert_contains(result, "mer.");
        assert_contains(result, "mars");
    });

    test("full composite date string", fn() {
        let dt = DateTime.parse("2024-03-06T14:30:00Z");
        let result = dt.format("%A %d %B %Y", "fr");
        assert_eq(result, "mercredi 06 mars 2024");
    });

    test("time-only format is unaffected by locale", fn() {
        let dt = DateTime.parse("2024-06-15T14:30:45Z");
        let en_result = dt.format("%H:%M:%S");
        let fr_result = dt.format("%H:%M:%S", "fr");
        assert_eq(en_result, fr_result);
    });
});

// ============================================================================
// Spanish locale
// ============================================================================
describe("DateTime.format() with Spanish locale", fn() {
    test("full month names", fn() {
        let months = [
            ["2024-01-15T00:00:00Z", "enero"],
            ["2024-02-15T00:00:00Z", "febrero"],
            ["2024-03-15T00:00:00Z", "marzo"],
            ["2024-04-15T00:00:00Z", "abril"],
            ["2024-05-15T00:00:00Z", "mayo"],
            ["2024-06-15T00:00:00Z", "junio"],
            ["2024-07-15T00:00:00Z", "julio"],
            ["2024-08-15T00:00:00Z", "agosto"],
            ["2024-09-15T00:00:00Z", "septiembre"],
            ["2024-10-15T00:00:00Z", "octubre"],
            ["2024-11-15T00:00:00Z", "noviembre"],
            ["2024-12-15T00:00:00Z", "diciembre"]
        ];
        months.each(fn(pair) {
            let dt = DateTime.parse(pair[0]);
            let result = dt.format("%B", "es");
            assert_eq(result, pair[1]);
        });
    });

    test("full day names", fn() {
        let days = [
            ["2024-01-01T00:00:00Z", "lunes"],
            ["2024-01-02T00:00:00Z", "martes"],
            ["2024-01-03T00:00:00Z", "miércoles"],
            ["2024-01-04T00:00:00Z", "jueves"],
            ["2024-01-05T00:00:00Z", "viernes"],
            ["2024-01-06T00:00:00Z", "sábado"],
            ["2024-01-07T00:00:00Z", "domingo"]
        ];
        days.each(fn(pair) {
            let dt = DateTime.parse(pair[0]);
            let result = dt.format("%A", "es");
            assert_eq(result, pair[1]);
        });
    });

    test("abbreviated month name (%b)", fn() {
        let dt = DateTime.parse("2024-09-15T10:30:00Z");
        let result = dt.format("%d %b %Y", "es");
        assert_contains(result, "sept.");
    });

    test("abbreviated day name (%a)", fn() {
        // 2024-03-06 is Wednesday
        let dt = DateTime.parse("2024-03-06T14:30:00Z");
        let result = dt.format("%a", "es");
        assert_eq(result, "mié.");
    });

    test("full composite date string", fn() {
        let dt = DateTime.parse("2024-03-06T14:30:00Z");
        let result = dt.format("%A %d %B %Y", "es");
        assert_eq(result, "miércoles 06 marzo 2024");
    });
});

// ============================================================================
// Numeric-only formats (locale should not affect output)
// ============================================================================
describe("DateTime.format() numeric formats with locale", fn() {
    test("ISO date format is same across locales", fn() {
        let dt = DateTime.parse("2024-01-15T10:30:00Z");
        let en = dt.format("%Y-%m-%d");
        let fr = dt.format("%Y-%m-%d", "fr");
        let es = dt.format("%Y-%m-%d", "es");
        assert_eq(en, fr);
        assert_eq(en, es);
    });

    test("time-only format is same across locales", fn() {
        let dt = DateTime.parse("2024-01-15T10:30:45Z");
        let en = dt.format("%H:%M:%S");
        let fr = dt.format("%H:%M:%S", "fr");
        let es = dt.format("%H:%M:%S", "es");
        assert_eq(en, fr);
        assert_eq(en, es);
    });

    test("numeric datetime format is same across locales", fn() {
        let dt = DateTime.parse("2024-06-15T14:30:00Z");
        let en = dt.format("%Y-%m-%d");
        let fr = dt.format("%Y-%m-%d", "fr");
        assert_eq(en, fr);
    });

    test("day-of-month and year numeric specifiers unaffected", fn() {
        let dt = DateTime.parse("2024-07-20T00:00:00Z");
        let result = dt.format("%d/%m/%Y", "fr");
        assert_contains(result, "20");
        assert_contains(result, "07");
        assert_contains(result, "2024");
    });
});

// ============================================================================
// Mixed formats (locale-sensitive + numeric parts)
// ============================================================================
describe("DateTime.format() mixed formats", fn() {
    test("month name with numeric day and year in fr", fn() {
        let dt = DateTime.parse("2024-07-20T00:00:00Z");
        let result = dt.format("%d %B %Y", "fr");
        assert_contains(result, "juillet");
        assert_contains(result, "20");
        assert_contains(result, "2024");
    });

    test("day name with full date in es", fn() {
        // 2024-07-20 is a Saturday
        let dt = DateTime.parse("2024-07-20T00:00:00Z");
        let result = dt.format("%A, %d %B %Y", "es");
        assert_contains(result, "sábado");
        assert_contains(result, "julio");
    });

    test("abbreviated day + full month in fr", fn() {
        let dt = DateTime.parse("2024-01-01T00:00:00Z");
        let result = dt.format("%a %d %B", "fr");
        assert_contains(result, "lun.");
        assert_contains(result, "janvier");
    });

    test("format with localized names preserves surrounding text", fn() {
        let dt = DateTime.parse("2024-03-06T14:30:00Z");
        let result = dt.format("%A %d %B %Y", "fr");
        assert_contains(result, "mercredi");
        assert_contains(result, "mars");
        assert_contains(result, "06");
        assert_contains(result, "2024");
    });

    test("format with abbreviated month and day in es", fn() {
        // 2024-01-01 is Monday
        let dt = DateTime.parse("2024-01-01T00:00:00Z");
        let result = dt.format("%a %d %b %Y", "es");
        assert_contains(result, "lun.");
        assert_contains(result, "ene.");
        assert_contains(result, "2024");
    });
});

// ============================================================================
// Edge cases
// ============================================================================
describe("DateTime.format() locale edge cases", fn() {
    test("unknown locale falls back to English names", fn() {
        let dt = DateTime.parse("2024-01-01T00:00:00Z");
        let result = dt.format("%A %B", "xx");
        assert_eq(result, "Monday January");
    });

    test("format on epoch date with locale", fn() {
        let dt = DateTime.epoch();
        let result = dt.format("%B %Y", "fr");
        assert_contains(result, "janvier");
        assert_contains(result, "1970");
    });

    test("format on DateTime.now() with locale does not crash", fn() {
        let dt = DateTime.now();
        let result_fr = dt.format("%A %d %B %Y", "fr");
        let result_es = dt.format("%A %d %B %Y", "es");
        assert_not_null(result_fr);
        assert_not_null(result_es);
        assert(len(result_fr) > 0);
        assert(len(result_es) > 0);
    });

    test("format on DateTime.utc() with locale does not crash", fn() {
        let dt = DateTime.utc();
        let result = dt.format("%B", "fr");
        assert_not_null(result);
        assert(len(result) > 0);
    });

    test("format on DateTime.from_unix() with locale", fn() {
        let dt = DateTime.from_unix(1704067200);
        let result = dt.format("%B %Y", "es");
        assert_contains(result, "enero");
        assert_contains(result, "2024");
    });

    test("same date with different locales produces different results", fn() {
        let dt = DateTime.parse("2024-02-15T10:30:00Z");
        let en = dt.format("%B", "en");
        let fr = dt.format("%B", "fr");
        let es = dt.format("%B", "es");
        assert_eq(en, "February");
        assert_eq(fr, "février");
        assert_eq(es, "febrero");
    });

    test("UTF-8 accented characters in locale output", fn() {
        let dt = DateTime.parse("2024-02-15T10:30:00Z");
        // French février has accent
        let fr = dt.format("%B", "fr");
        assert_eq(fr, "février");
        // Spanish miércoles has accent
        let dt2 = DateTime.parse("2024-01-03T00:00:00Z");
        let es = dt2.format("%A", "es");
        assert_eq(es, "miércoles");
    });

    test("December in all three locales", fn() {
        let dt = DateTime.parse("2024-12-25T00:00:00Z");
        assert_eq(dt.format("%B", "en"), "December");
        assert_eq(dt.format("%B", "fr"), "décembre");
        assert_eq(dt.format("%B", "es"), "diciembre");
    });
});

// ============================================================================
// Error handling
// ============================================================================
describe("DateTime.format() locale error handling", fn() {
    test("non-string locale throws error", fn() {
        let dt = DateTime.parse("2024-01-15T00:00:00Z");
        let error_thrown = false;
        try {
            dt.format("%Y-%m-%d", 42);
        } catch (e) {
            error_thrown = true;
            assert_contains(str(e), "locale must be a string");
        }
        assert_eq(error_thrown, true);
    });

    test("format without arguments throws error", fn() {
        let dt = DateTime.parse("2024-01-15T00:00:00Z");
        let error_thrown = false;
        try {
            dt.format();
        } catch (e) {
            error_thrown = true;
        }
        assert_eq(error_thrown, true);
    });
});
