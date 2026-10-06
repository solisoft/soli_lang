# File imports: `import "./x.sl"` (all exports), `import { a, b as c } from`,
# directory modules, transitive and diamond imports. An import carries the
# module's private declarations too, but never its top-level statements.

import "./_fixtures/geometry.sl"
import "./_fixtures/geometry.sl"
import "./_fixtures/shouting.sl"
import "./_fixtures/whispering.sl"
import "./_fixtures/shapes"
import { to_meters, to_centimeters as meters_to_cm, Ruler as Yardstick } from "./_fixtures/conversions"

class Tile implements Measurable
  def area
    1
  end
end

describe("import of a whole module") do
  context("exported declarations") do
    test("an exported function is callable by its name") do
      assert_eq(area(5), 25)
      assert_eq(perimeter(5), 20)
    end

    test("an exported const and let keep their values") do
      assert_eq(UNIT, "cm")
      assert_eq(default_side, 3)
      assert_eq(area(default_side), 9)
    end

    test("an exported class can be instantiated") do
      square = new Square(6)
      assert_eq(square.side, 6)
      assert_eq(square.area, 36)
      assert_eq(square.class, "Square")
    end

    test("an exported enum exposes its variants") do
      assert_eq(Orientation.Portrait, Orientation.Portrait)
      assert_ne(Orientation.Portrait, Orientation.Landscape)
    end

    test("an exported interface can be implemented") do
      assert_eq(new Tile().area, 1)
    end
  end

  context("private declarations") do
    test("an exported function calls a private helper beside it") do
      assert_eq(area(4), 16)
    end

    test("a private function and const are defined in the importer too") do
      assert_eq(_square(7), 49)
      assert_eq(SIDES_OF_SQUARE, 4)
    end

    test("top-level statements that declare nothing are not carried") do
      assert_not(defined("load_marker"))
    end
  end

  context("importing the same module twice") do
    test("the declarations are included once and still work") do
      assert_eq(area(2), 4)
      assert_eq(new Square(2).area, 4)
    end
  end
end

describe("named imports") do
  test("a named import binds the export under its own name") do
    assert_eq(to_meters(250), 2)
  end

  test("an alias binds the export under the new name") do
    assert_eq(meters_to_cm(3), 300)
  end

  test("an aliased class is instantiable under the alias") do
    assert_eq(new Yardstick().length, 30)
  end

  test("the original name of an aliased export stays defined") do
    assert_eq(to_centimeters(2), 200)
    assert_eq(new Ruler().length, 30)
  end

  test("the extension can be left off the path") do
    assert(defined("to_meters"))
    assert_eq(to_meters(100), 1)
  end
end

describe("modules that import other modules") do
  test("a module's own imports are available to its exports") do
    assert_eq(shout("ada"), "HELLO ADA")
  end

  test("a module's own imports are defined in the importer") do
    assert_eq(greet("ada"), "hello ada")
  end

  test("a module reached along two paths is included once") do
    assert_eq(shout("bob"), "HELLO BOB")
    assert_eq(whisper("BOB"), "hello bob...")
    assert_eq(Registry.label, "registry")
  end

  test("a directory import resolves to its mod.sl") do
    assert_eq(describe_circle(3), "shape:circle r=3 d=6")
  end

  test("a directory module's private helper and nested import are carried") do
    assert_eq(_describe("x"), "shape:x")
    assert_eq(diameter(5), 10)
  end
end
