# Date/time formatting helpers
#
# Helpers run in the application's environment, not a view's: the view-only
# `datetime_format` / `datetime_add_days` are not callable here ("Cannot call
# non-function value", a 500), so these go through DateTime.

def format_date(timestamp: Int, format: String) -> String
  DateTime.from_unix(timestamp).format(format)
end

def format_datetime(timestamp: Int) -> String
  DateTime.from_unix(timestamp).format("%Y-%m-%d %H:%M:%S")
end

def format_relative(timestamp: Int) -> String
  time_ago(timestamp)
end

def is_today(timestamp: Int) -> Bool
  DateTime.from_unix(timestamp).format("%Y-%m-%d") == DateTime.utc.format("%Y-%m-%d")
end
