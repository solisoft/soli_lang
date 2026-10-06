# The relation DSL (belongs_to, has_many, has_one, has_and_belongs_to_many)
# takes a symbol or a string, with or without parentheses. Each form registers
# its own relation name, and the query `includes` builds proves it registered.
#
# `soli fmt` adds parentheses to the bare forms: keep them bare, they are the
# point of the file.

class SpecBelongsToUser < Model
  belongs_to(:owner)
  belongs_to :bare_owner
  belongs_to("str_owner")
  belongs_to "str_owner2"
end

class SpecHasManyPosts < Model
  has_many(:items)
  has_many :bare_items
  has_many("str_items")
  has_many "str_items2"
end

class SpecHasOneProfile < Model
  has_one(:avatar)
  has_one :bare_avatar
  has_one "str_avatar"
end

class SpecHabtmTags < Model
  has_and_belongs_to_many(:categories)
  has_and_belongs_to_many :bare_categories
  has_and_belongs_to_many "str_categories"
end

describe("belongs_to") do
  test("belongs_to(:symbol)") do
    assert_eq(
      SpecBelongsToUser.includes("owner").to_query,
      "FOR doc IN spec_belongs_to_users LET _rel_owner = (FOR rel IN owners FILTER rel._key == doc.owner_id " +
        "LIMIT 1 RETURN rel) RETURN MERGE(doc, {owner: FIRST(_rel_owner)})"
    )
  end

  test("belongs_to :symbol without parentheses") do
    assert_contains(
      SpecBelongsToUser.includes("bare_owner").to_query,
      "FOR rel IN bare_owners FILTER rel._key == doc.bare_owner_id LIMIT 1"
    )
  end

  test("belongs_to(\"string\")") do
    assert_contains(
      SpecBelongsToUser.includes("str_owner").to_query,
      "FOR rel IN str_owners FILTER rel._key == doc.str_owner_id LIMIT 1"
    )
  end

  test("belongs_to \"string\" without parentheses") do
    assert_contains(
      SpecBelongsToUser.includes("str_owner2").to_query,
      "FOR rel IN str_owner2s FILTER rel._key == doc.str_owner2_id LIMIT 1"
    )
  end
end

describe("has_many") do
  test("has_many(:symbol)") do
    assert_eq(
      SpecHasManyPosts.includes("items").to_query,
      "FOR doc IN spec_has_many_posts LET _rel_items = (FOR rel IN items FILTER rel.spec_has_many_posts_id == " +
        "doc._key RETURN rel) RETURN MERGE(doc, {items: _rel_items})"
    )
  end

  test("has_many :symbol without parentheses") do
    assert_contains(
      SpecHasManyPosts.includes("bare_items").to_query,
      "FOR rel IN bare_items FILTER rel.spec_has_many_posts_id == doc._key RETURN rel"
    )
  end

  test("has_many(\"string\")") do
    assert_contains(
      SpecHasManyPosts.includes("str_items").to_query,
      "FOR rel IN str_items FILTER rel.spec_has_many_posts_id == doc._key RETURN rel"
    )
  end

  test("has_many \"string\" without parentheses") do
    assert_contains(
      SpecHasManyPosts.includes("str_items2").to_query,
      "FOR rel IN str_items2 FILTER rel.spec_has_many_posts_id == doc._key RETURN rel"
    )
  end
end

describe("has_one") do
  test("has_one(:symbol) limits to one row") do
    assert_eq(
      SpecHasOneProfile.includes("avatar").to_query,
      "FOR doc IN spec_has_one_profiles LET _rel_avatar = (FOR rel IN avatars FILTER rel.spec_has_one_profile_id " +
        "== doc._key LIMIT 1 RETURN rel) RETURN MERGE(doc, {avatar: FIRST(_rel_avatar)})"
    )
  end

  test("has_one :symbol without parentheses") do
    assert_contains(
      SpecHasOneProfile.includes("bare_avatar").to_query,
      "FOR rel IN bare_avatars FILTER rel.spec_has_one_profile_id == doc._key LIMIT 1"
    )
  end

  test("has_one \"string\" without parentheses") do
    assert_contains(
      SpecHasOneProfile.includes("str_avatar").to_query,
      "FOR rel IN str_avatars FILTER rel.spec_has_one_profile_id == doc._key LIMIT 1"
    )
  end
end

describe("has_and_belongs_to_many") do
  test("habtm(:symbol) goes through an alphabetical join table") do
    assert_eq(
      SpecHabtmTags.includes("categories").to_query,
      "FOR doc IN spec_habtm_tags LET _rel_categories = (FOR jt IN categories_spec_habtm_tags FILTER " +
        "jt.spec_habtm_tags_id == doc._key FOR rel IN categories FILTER rel._key == jt.category_id RETURN rel) " +
        "RETURN MERGE(doc, {categories: _rel_categories})"
    )
  end

  test("habtm :symbol without parentheses") do
    assert_contains(
      SpecHabtmTags.includes("bare_categories").to_query,
      "FOR jt IN bare_categories_spec_habtm_tags FILTER jt.spec_habtm_tags_id == doc._key"
    )
  end

  test("habtm \"string\" without parentheses") do
    assert_contains(
      SpecHabtmTags.includes("str_categories").to_query,
      "FOR jt IN spec_habtm_tags_str_categories FILTER jt.spec_habtm_tags_id == doc._key"
    )
  end
end

describe("includes") do
  test("raises on a relation that was never declared") do
    assert_raises("No relation 'nope' defined on SpecHasManyPosts") do
      SpecHasManyPosts.includes("nope").to_query
    end
  end
end
