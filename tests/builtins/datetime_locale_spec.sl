# DateTime#format with a locale: localized month and day names (%B %b %A %a),
# numeric specifiers untouched. Instants are read in the UTC view so the names
# hold in every $TZ.

# The 15th of each month of 2024, at noon UTC.
def mid_month(month)
  padded = month < 10 ? "0#{month}" : "#{month}"
  DateTime.parse("2024-#{padded}-15T12:00:00Z").utc
end

# 2024-01-01 is a Monday: day 1..7 is Monday..Sunday.
def week_day(day)
  DateTime.parse("2024-01-0#{day}T12:00:00Z").utc
end

def month_names(pattern, locale)
  (1..13).map { |month| mid_month(month).format(pattern, locale) }
end

def day_names(pattern, locale)
  (1..8).map { |day| week_day(day).format(pattern, locale) }
end

describe("format without a locale") do
  test("uses English names") do
    assert_eq(week_day(1).format("%A"), "Monday")
    assert_eq(mid_month(2).format("%B"), "February")
  end

  test("matches the en locale") do
    moment = DateTime.parse("2024-06-15T10:30:00Z").utc
    assert_eq(moment.format("%A %d %B %Y"), "Saturday 15 June 2024")
    assert_eq(moment.format("%A %d %B %Y", "en"), "Saturday 15 June 2024")
  end
end

describe("French") do
  test("month names") do
    assert_eq(month_names("%B", "fr"), [
      "janvier",
      "février",
      "mars",
      "avril",
      "mai",
      "juin",
      "juillet",
      "août",
      "septembre",
      "octobre",
      "novembre",
      "décembre"
    ])
  end

  test("abbreviated month names") do
    assert_eq(month_names("%b", "fr"), [
      "janv.",
      "févr.",
      "mars",
      "avr.",
      "mai",
      "juin",
      "juil.",
      "août",
      "sept.",
      "oct.",
      "nov.",
      "déc."
    ])
  end

  test("day names") do
    assert_eq(day_names("%A", "fr"), ["lundi", "mardi", "mercredi", "jeudi", "vendredi", "samedi", "dimanche"])
  end

  test("abbreviated day names") do
    assert_eq(day_names("%a", "fr"), ["lun.", "mar.", "mer.", "jeu.", "ven.", "sam.", "dim."])
  end

  test("a composite date") do
    moment = DateTime.parse("2024-03-06T14:30:00Z").utc
    assert_eq(moment.format("%A %d %B %Y", "fr"), "mercredi 06 mars 2024")
    assert_eq(moment.format("%a %d %B", "fr"), "mer. 06 mars")
    assert_eq(mid_month(2).format("%d %b %Y", "fr"), "15 févr. 2024")
  end
end

describe("Spanish") do
  test("month names") do
    assert_eq(month_names("%B", "es"), [
      "enero",
      "febrero",
      "marzo",
      "abril",
      "mayo",
      "junio",
      "julio",
      "agosto",
      "septiembre",
      "octubre",
      "noviembre",
      "diciembre"
    ])
  end

  test("abbreviated month names") do
    assert_eq(month_names("%b", "es"), [
      "ene.",
      "feb.",
      "mar.",
      "abr.",
      "mayo",
      "jun.",
      "jul.",
      "ago.",
      "sept.",
      "oct.",
      "nov.",
      "dic."
    ])
  end

  test("day names") do
    assert_eq(day_names("%A", "es"), ["lunes", "martes", "miércoles", "jueves", "viernes", "sábado", "domingo"])
  end

  test("abbreviated day names") do
    assert_eq(day_names("%a", "es"), ["lun.", "mar.", "mié.", "jue.", "vie.", "sáb.", "dom."])
  end

  test("a composite date") do
    assert_eq(DateTime.parse("2024-03-06T14:30:00Z").utc.format("%A %d %B %Y", "es"), "miércoles 06 marzo 2024")
    assert_eq(DateTime.parse("2024-07-20T12:00:00Z").utc.format("%A, %d %B %Y", "es"), "sábado, 20 julio 2024")
    assert_eq(week_day(1).format("%a %d %b %Y", "es"), "lun. 01 ene. 2024")
  end
end

describe("German, Italian and Portuguese") do
  test("German names") do
    assert_eq(week_day(1).format("%A %B", "de"), "Montag Januar")
    assert_eq(mid_month(3).format("%b", "de"), "März")
  end

  test("Italian names") do
    assert_eq(week_day(1).format("%A %B", "it"), "lunedì gennaio")
  end

  test("Portuguese names") do
    assert_eq(week_day(1).format("%A %B", "pt"), "segunda-feira janeiro")
  end
end

describe("numeric specifiers") do
  test("are the same in every locale") do
    moment = DateTime.parse("2024-07-20T14:30:45Z").utc
    ["en", "fr", "es", "de"].each do |locale|
      assert_eq(moment.format("%Y-%m-%d %H:%M:%S", locale), "2024-07-20 14:30:45")
      assert_eq(moment.format("%d/%m/%Y", locale), "20/07/2024")
    end
  end

  test("surround a localized name unchanged") do
    assert_eq(DateTime.parse("2024-07-20T12:00:00Z").utc.format("%d %B %Y", "fr"), "20 juillet 2024")
    assert_eq(DateTime.epoch.utc.format("%B %Y", "fr"), "janvier 1970")
    assert_eq(DateTime.from_unix(1704067200).utc.format("%B %Y", "es"), "enero 2024")
  end
end

describe("locale edge cases") do
  test("an unknown locale falls back to English") do
    assert_eq(week_day(1).format("%A %B", "xx"), "Monday January")
    assert_eq(week_day(1).format("%A %B", ""), "Monday January")
  end

  test("locale codes are exact: a region or upper case falls back to English") do
    assert_eq(week_day(1).format("%A %B", "FR"), "Monday January")
    assert_eq(week_day(1).format("%A %B", "fr-FR"), "Monday January")
  end

  test("the local view is localized too") do
    moment = DateTime.now
    assert_eq(moment.format("%B", "fr"), mid_month(moment.month).format("%B", "fr"))
  end

  test("a non-string locale raises") do
    assert_raises("locale must be a string") do
      week_day(1).format("%Y-%m-%d", [42][0])
    end
  end
end
