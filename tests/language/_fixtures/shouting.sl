# Fixture for imports_spec.sl: builds on greetings.sl.
import "./greetings.sl"

export def shout(name)
  greet(name).upcase
end
