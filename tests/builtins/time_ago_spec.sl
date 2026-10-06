# time_ago(timestamp): "N units ago" for a Unix timestamp or a date string,
# phrased in the current I18n locale.
#
# time_ago reads the wall clock and ignores freeze_time (see the pending test
# at the end), so `now` is read fresh before each test; the freeze pins it once
# time_ago honours it. The minute buckets and above cannot tick over meanwhile.

now = 0

def ago(seconds)
  time_ago(now - seconds)
end

describe("time_ago") do
  before_each() do
    now = datetime_now()
    freeze_time(now)
    set_locale("en")
  end

  # The locale is process-wide and outlives this file.
  after_each() do
    set_locale("en")
  end

  describe("English") do
    test("seconds ago") do
      assert_eq(ago(30), "30 seconds ago")
    end

    test("1 second ago (singular)") do
      assert_eq(ago(1), "1 second ago")
    end

    test("minutes ago") do
      assert_eq(ago(300), "5 minutes ago")
    end

    test("1 minute ago (singular)") do
      assert_eq(ago(60), "1 minute ago")
    end

    test("hours ago") do
      assert_eq(ago(7200), "2 hours ago")
    end

    test("1 hour ago (singular)") do
      assert_eq(ago(3600), "1 hour ago")
    end

    test("days ago") do
      assert_eq(ago(259200), "3 days ago")
    end

    test("1 day ago (singular)") do
      assert_eq(ago(86400), "1 day ago")
    end

    test("weeks ago") do
      assert_eq(ago(604800 * 2), "2 weeks ago")
    end

    test("1 week ago (singular)") do
      assert_eq(ago(604800), "1 week ago")
    end

    test("months ago") do
      assert_eq(ago(2592000 * 6), "6 months ago")
    end

    test("1 month ago (singular)") do
      assert_eq(ago(2592000), "1 month ago")
    end

    test("years ago") do
      assert_eq(ago(31536000 * 10), "10 years ago")
    end

    test("1 year ago (singular)") do
      assert_eq(ago(31536000), "1 year ago")
    end

    test("future timestamp") do
      assert_eq(time_ago(now + 3600), "in the future")
    end
  end

  describe("unit boundaries") do
    test("the current second is 0 seconds ago") do
      assert_eq(ago(0), "0 seconds ago")
    end

    test("each unit runs up to the next one") do
      assert_eq(ago(59), "59 seconds ago")
      assert_eq(ago(3599), "59 minutes ago")
      assert_eq(ago(86399), "23 hours ago")
      assert_eq(ago(604799), "6 days ago")
      assert_eq(ago(2591999), "4 weeks ago")
      assert_eq(ago(31535999), "12 months ago")
    end

    test("one second into the future is the future") do
      assert_eq(time_ago(now + 1), "in the future")
    end
  end

  describe("French") do
    before_each() do
      set_locale("fr")
    end

    test("seconds ago") do
      assert_eq(ago(30), "il y a 30 secondes")
    end

    test("1 second ago (singular)") do
      assert_eq(ago(1), "il y a 1 seconde")
    end

    test("minutes ago") do
      assert_eq(ago(300), "il y a 5 minutes")
    end

    test("1 minute ago (singular)") do
      assert_eq(ago(60), "il y a 1 minute")
    end

    test("hours ago") do
      assert_eq(ago(7200), "il y a 2 heures")
    end

    test("1 hour ago (singular)") do
      assert_eq(ago(3600), "il y a 1 heure")
    end

    test("days ago") do
      assert_eq(ago(259200), "il y a 3 jours")
    end

    test("1 day ago (singular)") do
      assert_eq(ago(86400), "il y a 1 jour")
    end

    test("weeks ago") do
      assert_eq(ago(604800 * 2), "il y a 2 semaines")
    end

    test("1 week ago (singular)") do
      assert_eq(ago(604800), "il y a 1 semaine")
    end

    test("months ago") do
      assert_eq(ago(2592000 * 6), "il y a 6 mois")
    end

    test("1 month ago (singular)") do
      assert_eq(ago(2592000), "il y a 1 mois")
    end

    test("years ago") do
      assert_eq(ago(31536000 * 10), "il y a 10 ans")
    end

    test("1 year ago (singular)") do
      assert_eq(ago(31536000), "il y a 1 an")
    end

    test("future timestamp") do
      assert_eq(time_ago(now + 3600), "dans le futur")
    end
  end

  describe("German") do
    before_each() do
      set_locale("de")
    end

    test("seconds ago") do
      assert_eq(ago(30), "vor 30 Sekunden")
      assert_eq(ago(1), "vor 1 Sekunde")
    end

    test("minutes ago") do
      assert_eq(ago(300), "vor 5 Minuten")
    end

    test("1 hour ago (singular)") do
      assert_eq(ago(3600), "vor 1 Stunde")
    end

    test("days ago") do
      assert_eq(ago(259200), "vor 3 Tagen")
    end

    test("weeks, months and years ago") do
      assert_eq(ago(604800 * 2), "vor 2 Wochen")
      assert_eq(ago(2592000 * 6), "vor 6 Monaten")
      assert_eq(ago(31536000 * 10), "vor 10 Jahren")
    end

    test("future timestamp") do
      assert_eq(time_ago(now + 3600), "in der Zukunft")
    end
  end

  describe("Spanish") do
    before_each() do
      set_locale("es")
    end

    test("minutes ago") do
      assert_eq(ago(300), "hace 5 minutos")
    end

    test("1 hour ago (singular)") do
      assert_eq(ago(3600), "hace 1 hora")
    end

    test("days ago with accent") do
      assert_eq(ago(259200), "hace 3 días")
    end

    test("future timestamp") do
      assert_eq(time_ago(now + 3600), "en el futuro")
    end
  end

  describe("Italian") do
    before_each() do
      set_locale("it")
    end

    test("minutes ago") do
      assert_eq(ago(300), "5 minuti fa")
    end

    test("1 hour ago (singular)") do
      assert_eq(ago(3600), "1 ora fa")
    end

    test("days ago") do
      assert_eq(ago(259200), "3 giorni fa")
    end

    test("future timestamp") do
      assert_eq(time_ago(now + 3600), "nel futuro")
    end
  end

  describe("Portuguese") do
    before_each() do
      set_locale("pt")
    end

    test("minutes ago") do
      assert_eq(ago(300), "há 5 minutos")
    end

    test("1 hour ago (singular)") do
      assert_eq(ago(3600), "há 1 hora")
    end

    test("future timestamp") do
      assert_eq(time_ago(now + 3600), "no futuro")
    end
  end

  describe("Japanese") do
    before_each() do
      set_locale("ja")
    end

    test("minutes ago") do
      assert_eq(ago(300), "5分前")
    end

    test("1 hour ago (singular)") do
      assert_eq(ago(3600), "1時間前")
    end

    test("days and years ago") do
      assert_eq(ago(259200), "3日前")
      assert_eq(ago(31536000 * 10), "10年前")
    end

    test("future timestamp") do
      assert_eq(time_ago(now + 3600), "未来")
    end
  end

  describe("Chinese") do
    before_each() do
      set_locale("zh")
    end

    test("minutes ago") do
      assert_eq(ago(300), "5分钟前")
    end

    test("1 hour ago (singular)") do
      assert_eq(ago(3600), "1小时前")
    end

    test("future timestamp") do
      assert_eq(time_ago(now + 3600), "未来")
    end
  end

  describe("locale switching") do
    test("the same timestamp follows the current locale") do
      past = now - 300
      set_locale("en")
      english = time_ago(past)
      set_locale("fr")
      french = time_ago(past)
      set_locale("es")
      spanish = time_ago(past)
      assert_eq(english, "5 minutes ago")
      assert_eq(french, "il y a 5 minutes")
      assert_eq(spanish, "hace 5 minutos")
    end

    test("an unknown locale falls back to English") do
      set_locale("xx")
      assert_eq(ago(300), "5 minutes ago")
    end
  end

  describe("input") do
    test("accepts an RFC 3339 date string") do
      assert_eq(time_ago(DateTime.from_unix(now - 7200).to_iso), "2 hours ago")
    end

    test("accepts a date string without an offset") do
      assert_eq(time_ago(DateTime.from_unix(now - 7200).utc.to_string), "2 hours ago")
    end

    test("raises on a DateTime") do
      assert_raises("time_ago() expects timestamp (int) or date string, got DateTime") do
        time_ago(DateTime.from_unix(now))
      end
    end

    test("raises on nil") do
      assert_raises("got null") do
        time_ago(nil)
      end
    end

    test("raises on a string that is not a date") do
      pending("bug: time_ago(\"garbage\") reads the string as the epoch and returns \"56 years ago\"")
      assert_raises() do
        time_ago("garbage")
      end
    end

    test("measures against the frozen clock") do
      pending("bug: time_ago reads Utc::now and ignores freeze_time")
      freeze_time(1700000000)
      assert_eq(time_ago(1700000000 - 30), "30 seconds ago")
    end
  end
end
