# A subclass of a single-table-inheritance hierarchy deletes rows of its own
# kind only — also through `Model.delete(id)` on a subclass that declares
# dependents, which loads the row itself before it cascades. Runs against
# SoliDB.

class StiPet < Model
end

class StiPuppy < StiPet
  has_many("sti_toys", dependent: "delete")
end

class StiKitten < StiPet
end

class StiToy < Model
  belongs_to("sti_puppy")
end

describe("Model.delete(id) on a subclass with dependents") do
  before_each() do
    requires_solidb()
    StiToy.delete_all
    StiPet.delete_all
  end

  test("another subclass's row is refused, and stays") do
    kitten = StiKitten.create({"name": "Tom"})
    answer = StiPuppy.delete(kitten._key)
    assert_eq(type(answer), "string")
    assert_contains(answer, "not found")
    assert_eq(StiKitten.find_by("name", "Tom")._key, kitten._key)
  end

  test("its own row goes, with what depends on it") do
    puppy = StiPuppy.create({"name": "Rex"})
    StiToy.create({"sti_puppy_id": puppy._key, "name": "ball"})
    StiPuppy.delete(puppy._key)
    assert_null(StiPuppy.find_by("name", "Rex"))
    assert_eq(StiToy.count, 0)
  end
end
