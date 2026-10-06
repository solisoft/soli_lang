# Fixture for imports_spec.sl: a directory module, imported by its folder name.
import "./circle.sl"

def _describe(name)
  "shape:#{name}"
end

export def describe_circle(radius)
  _describe("circle r=#{radius} d=#{diameter(radius)}")
end
