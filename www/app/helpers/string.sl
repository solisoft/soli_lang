# String manipulation helpers

def truncate(text: String, length: Int, suffix: String) -> String
  return text if text.length <= length

  text.substring(0, length - suffix.length) + suffix
end

def truncate_default(text: String, length: Int) -> String
  truncate(text, length, "...")
end

def slugify(text: String) -> String
  sanitize_html(text.downcase.replace("_", "-").replace(" ", "-"))
end

def titleize(text: String) -> String
  text.split(" ").map { |w| w.length > 0 ? w.substring(0, 1).upcase + w.substring(1, w.length) : w }.join(" ")
end
