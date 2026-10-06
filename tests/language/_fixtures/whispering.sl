# Fixture for imports_spec.sl: also imports greetings.sl (diamond).
import "./greetings.sl"

export def whisper(name)
  "#{greet(name).downcase}..."
end
