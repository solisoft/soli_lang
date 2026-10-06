# Fixture for imports_spec.sl: imported by name only, some under an alias.

export def to_meters(centimeters)
  centimeters / 100
end

export def to_centimeters(meters)
  meters * 100
end

export class Ruler
  def length
    30
  end
end
