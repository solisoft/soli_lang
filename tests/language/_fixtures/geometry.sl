# Fixture for imports_spec.sl: exported and private declarations side by side.

const SIDES_OF_SQUARE = 4

def _square(n)
  n * n
end

load_marker = "top-level code is not carried into the importer"

export const UNIT = "cm"

export let default_side = 3

export def area(side)
  _square(side)
end

export def perimeter(side)
  side * SIDES_OF_SQUARE
end

export class Square
  side: Int

  new(side: Int)
    @side = side
  end

  def area
    _square(@side)
  end
end

export enum Orientation
  Portrait,
  Landscape
end

export interface Measurable {
  fn area()
}
