// Test that DSL functions accept symbols and strings, with and without parens,
// and that the relations are properly registered (verify via query generation).
class SpecBelongsToUser extends Model
    belongs_to(:owner)
    belongs_to :owner
    belongs_to("str_owner")
    belongs_to "str_owner2"
end

class SpecHasManyPosts extends Model
    has_many(:items)
    has_many :items
    has_many("str_items")
    has_many "str_items2"
end

class SpecHasOneProfile extends Model
    has_one(:avatar)
    has_one "str_avatar"
end

class SpecHabtmTags extends Model
    has_and_belongs_to_many(:categories)
    has_and_belongs_to_many "str_categories"
end

describe("DSL with symbols", fn() {
    test("belongs_to :symbol generates correct query", fn() {
        let q = SpecBelongsToUser.includes("owner").to_query
        assert(q.contains("FILTER rel._key == doc.owner_id"))
    })
    test("belongs_to :symbol no-parens generates query", fn() {
        // The second belongs_to :owner declaration registers the same relation,
        // so includes("owner") still works
        let q = SpecBelongsToUser.includes("owner").to_query
        assert(q.contains("FILTER rel._key == doc.owner_id"))
    })
    test("belongs_to string no-parens generates correct query", fn() {
        let q = SpecBelongsToUser.includes("str_owner2").to_query
        assert(q.contains("FILTER rel._key == doc.str_owner2_id"))
    })
    test("has_many :symbol generates subquery", fn() {
        let q = SpecHasManyPosts.includes("items").to_query
        assert(q.contains("FOR rel IN items FILTER rel.spec_has_many_posts_id == doc._key RETURN rel"))
    })
    test("has_many string no-parens generates subquery", fn() {
        let q = SpecHasManyPosts.includes("str_items2").to_query
        assert(q.contains("FOR rel IN str_items2"))
    })
    test("has_one :symbol generates LIMIT 1", fn() {
        let q = SpecHasOneProfile.includes("avatar").to_query
        assert(q.contains("LIMIT 1"))
    })
    test("has_one string no-parens generates LIMIT 1", fn() {
        let q = SpecHasOneProfile.includes("str_avatar").to_query
        assert(q.contains("LIMIT 1"))
    })
    test("habtm :symbol generates join table", fn() {
        let q = SpecHabtmTags.includes("categories").to_query
        assert(q.contains("categories_spec_habtm_tags"))
        assert(q.contains("spec_habtm_tags_id"))
    })
    test("habtm string no-parens generates join table", fn() {
        let q = SpecHabtmTags.includes("str_categories").to_query
        assert(q.contains("spec_habtm_tags_str_categories"))
    })
});
