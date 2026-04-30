// ============================================================================
// time_ago I18n Test Suite
// ============================================================================
// Integration tests for time_ago() builtin with locale support
// ============================================================================

describe("time_ago I18n", fn() {
    before_each(fn() {
        set_locale("en");
    });

    // ========================================================================
    // English locale
    // ========================================================================
    describe("English locale", fn() {
        test("seconds ago", fn() {
            let now = DateTime.now().to_unix();
            let result = time_ago(now - 30);
            assert_eq(result, "30 seconds ago");
        });

        test("1 second ago (singular)", fn() {
            let now = DateTime.now().to_unix();
            let result = time_ago(now - 1);
            assert_eq(result, "1 second ago");
        });

        test("minutes ago", fn() {
            let now = DateTime.now().to_unix();
            let result = time_ago(now - 300);
            assert_eq(result, "5 minutes ago");
        });

        test("1 minute ago (singular)", fn() {
            let now = DateTime.now().to_unix();
            let result = time_ago(now - 60);
            assert_eq(result, "1 minute ago");
        });

        test("hours ago", fn() {
            let now = DateTime.now().to_unix();
            let result = time_ago(now - 7200);
            assert_eq(result, "2 hours ago");
        });

        test("1 hour ago (singular)", fn() {
            let now = DateTime.now().to_unix();
            let result = time_ago(now - 3600);
            assert_eq(result, "1 hour ago");
        });

        test("days ago", fn() {
            let now = DateTime.now().to_unix();
            let result = time_ago(now - 259200);
            assert_eq(result, "3 days ago");
        });

        test("1 day ago (singular)", fn() {
            let now = DateTime.now().to_unix();
            let result = time_ago(now - 86400);
            assert_eq(result, "1 day ago");
        });

        test("weeks ago", fn() {
            let now = DateTime.now().to_unix();
            let result = time_ago(now - 604800 * 2);
            assert_eq(result, "2 weeks ago");
        });

        test("1 week ago (singular)", fn() {
            let now = DateTime.now().to_unix();
            let result = time_ago(now - 604800);
            assert_eq(result, "1 week ago");
        });

        test("months ago", fn() {
            let now = DateTime.now().to_unix();
            let result = time_ago(now - 2592000 * 6);
            assert_eq(result, "6 months ago");
        });

        test("1 month ago (singular)", fn() {
            let now = DateTime.now().to_unix();
            let result = time_ago(now - 2592000);
            assert_eq(result, "1 month ago");
        });

        test("years ago", fn() {
            let now = DateTime.now().to_unix();
            let result = time_ago(now - 31536000 * 10);
            assert_eq(result, "10 years ago");
        });

        test("1 year ago (singular)", fn() {
            let now = DateTime.now().to_unix();
            let result = time_ago(now - 31536000);
            assert_eq(result, "1 year ago");
        });

        test("future timestamp", fn() {
            let now = DateTime.now().to_unix();
            let result = time_ago(now + 3600);
            assert_eq(result, "in the future");
        });
    });

    // ========================================================================
    // French locale
    // ========================================================================
    describe("French locale", fn() {
        before_each(fn() {
            set_locale("fr");
        });

        test("seconds ago", fn() {
            let now = DateTime.now().to_unix();
            let result = time_ago(now - 30);
            assert_eq(result, "il y a 30 secondes");
        });

        test("1 second ago (singular)", fn() {
            let now = DateTime.now().to_unix();
            let result = time_ago(now - 1);
            assert_eq(result, "il y a 1 seconde");
        });

        test("minutes ago", fn() {
            let now = DateTime.now().to_unix();
            let result = time_ago(now - 300);
            assert_eq(result, "il y a 5 minutes");
        });

        test("1 minute ago (singular)", fn() {
            let now = DateTime.now().to_unix();
            let result = time_ago(now - 60);
            assert_eq(result, "il y a 1 minute");
        });

        test("hours ago", fn() {
            let now = DateTime.now().to_unix();
            let result = time_ago(now - 7200);
            assert_eq(result, "il y a 2 heures");
        });

        test("1 hour ago (singular)", fn() {
            let now = DateTime.now().to_unix();
            let result = time_ago(now - 3600);
            assert_eq(result, "il y a 1 heure");
        });

        test("days ago", fn() {
            let now = DateTime.now().to_unix();
            let result = time_ago(now - 259200);
            assert_eq(result, "il y a 3 jours");
        });

        test("1 day ago (singular)", fn() {
            let now = DateTime.now().to_unix();
            let result = time_ago(now - 86400);
            assert_eq(result, "il y a 1 jour");
        });

        test("weeks ago", fn() {
            let now = DateTime.now().to_unix();
            let result = time_ago(now - 604800 * 2);
            assert_eq(result, "il y a 2 semaines");
        });

        test("1 week ago (singular)", fn() {
            let now = DateTime.now().to_unix();
            let result = time_ago(now - 604800);
            assert_eq(result, "il y a 1 semaine");
        });

        test("months ago", fn() {
            let now = DateTime.now().to_unix();
            let result = time_ago(now - 2592000 * 6);
            assert_eq(result, "il y a 6 mois");
        });

        test("1 month ago (singular)", fn() {
            let now = DateTime.now().to_unix();
            let result = time_ago(now - 2592000);
            assert_eq(result, "il y a 1 mois");
        });

        test("years ago", fn() {
            let now = DateTime.now().to_unix();
            let result = time_ago(now - 31536000 * 10);
            assert_eq(result, "il y a 10 ans");
        });

        test("1 year ago (singular)", fn() {
            let now = DateTime.now().to_unix();
            let result = time_ago(now - 31536000);
            assert_eq(result, "il y a 1 an");
        });

        test("future timestamp", fn() {
            let now = DateTime.now().to_unix();
            let result = time_ago(now + 3600);
            assert_eq(result, "dans le futur");
        });
    });

    // ========================================================================
    // German locale
    // ========================================================================
    describe("German locale", fn() {
        before_each(fn() {
            set_locale("de");
        });

        test("minutes ago", fn() {
            let now = DateTime.now().to_unix();
            let result = time_ago(now - 300);
            assert_eq(result, "vor 5 Minuten");
        });

        test("1 hour ago (singular)", fn() {
            let now = DateTime.now().to_unix();
            let result = time_ago(now - 3600);
            assert_eq(result, "vor 1 Stunde");
        });

        test("days ago", fn() {
            let now = DateTime.now().to_unix();
            let result = time_ago(now - 259200);
            assert_eq(result, "vor 3 Tagen");
        });

        test("future timestamp", fn() {
            let now = DateTime.now().to_unix();
            let result = time_ago(now + 3600);
            assert_eq(result, "in der Zukunft");
        });
    });

    // ========================================================================
    // Spanish locale
    // ========================================================================
    describe("Spanish locale", fn() {
        before_each(fn() {
            set_locale("es");
        });

        test("minutes ago", fn() {
            let now = DateTime.now().to_unix();
            let result = time_ago(now - 300);
            assert_eq(result, "hace 5 minutos");
        });

        test("1 hour ago (singular)", fn() {
            let now = DateTime.now().to_unix();
            let result = time_ago(now - 3600);
            assert_eq(result, "hace 1 hora");
        });

        test("days ago with accent", fn() {
            let now = DateTime.now().to_unix();
            let result = time_ago(now - 259200);
            assert_eq(result, "hace 3 días");
        });

        test("future timestamp", fn() {
            let now = DateTime.now().to_unix();
            let result = time_ago(now + 3600);
            assert_eq(result, "en el futuro");
        });
    });

    // ========================================================================
    // Italian locale
    // ========================================================================
    describe("Italian locale", fn() {
        before_each(fn() {
            set_locale("it");
        });

        test("minutes ago", fn() {
            let now = DateTime.now().to_unix();
            let result = time_ago(now - 300);
            assert_eq(result, "5 minuti fa");
        });

        test("1 hour ago (singular)", fn() {
            let now = DateTime.now().to_unix();
            let result = time_ago(now - 3600);
            assert_eq(result, "1 ora fa");
        });

        test("days ago", fn() {
            let now = DateTime.now().to_unix();
            let result = time_ago(now - 259200);
            assert_eq(result, "3 giorni fa");
        });

        test("future timestamp", fn() {
            let now = DateTime.now().to_unix();
            let result = time_ago(now + 3600);
            assert_eq(result, "nel futuro");
        });
    });

    // ========================================================================
    // Portuguese locale
    // ========================================================================
    describe("Portuguese locale", fn() {
        before_each(fn() {
            set_locale("pt");
        });

        test("minutes ago", fn() {
            let now = DateTime.now().to_unix();
            let result = time_ago(now - 300);
            assert_eq(result, "há 5 minutos");
        });

        test("1 hour ago (singular)", fn() {
            let now = DateTime.now().to_unix();
            let result = time_ago(now - 3600);
            assert_eq(result, "há 1 hora");
        });

        test("future timestamp", fn() {
            let now = DateTime.now().to_unix();
            let result = time_ago(now + 3600);
            assert_eq(result, "no futuro");
        });
    });

    // ========================================================================
    // Japanese locale
    // ========================================================================
    describe("Japanese locale", fn() {
        before_each(fn() {
            set_locale("ja");
        });

        test("minutes ago", fn() {
            let now = DateTime.now().to_unix();
            let result = time_ago(now - 300);
            assert_eq(result, "5分前");
        });

        test("1 hour ago (singular)", fn() {
            let now = DateTime.now().to_unix();
            let result = time_ago(now - 3600);
            assert_eq(result, "1時間前");
        });

        test("future timestamp", fn() {
            let now = DateTime.now().to_unix();
            let result = time_ago(now + 3600);
            assert_eq(result, "未来");
        });
    });

    // ========================================================================
    // Chinese locale
    // ========================================================================
    describe("Chinese locale", fn() {
        before_each(fn() {
            set_locale("zh");
        });

        test("minutes ago", fn() {
            let now = DateTime.now().to_unix();
            let result = time_ago(now - 300);
            assert_eq(result, "5分钟前");
        });

        test("1 hour ago (singular)", fn() {
            let now = DateTime.now().to_unix();
            let result = time_ago(now - 3600);
            assert_eq(result, "1小时前");
        });

        test("future timestamp", fn() {
            let now = DateTime.now().to_unix();
            let result = time_ago(now + 3600);
            assert_eq(result, "未来");
        });
    });

    // ========================================================================
    // Locale switching
    // ========================================================================
    describe("Locale switching", fn() {
        test("changing locale affects time_ago output", fn() {
            let now = DateTime.now().to_unix();
            let past = now - 300;

            set_locale("en");
            let en = time_ago(past);

            set_locale("fr");
            let fr = time_ago(past);

            set_locale("es");
            let es = time_ago(past);

            assert_eq(en, "5 minutes ago");
            assert_eq(fr, "il y a 5 minutes");
            assert_eq(es, "hace 5 minutos");
        });
    });

    // ========================================================================
    // String timestamp parsing
    // ========================================================================
    describe("String timestamp input", fn() {
        test("accepts integer timestamp", fn() {
            let ts = 1704067200;
            let result = time_ago(ts);
            assert(len(result) > 0);
        });
    });
});
