# ============================================================================
# Single-collection inheritance (STI): a model inheriting from another model
# shares its base's collection with a `type` discriminator.
#   - subclass writes stamp type; rows hydrate as their stored type
#   - subclass queries are type-scoped (including descendants); the base
#     class matches every row, Rails-style
#   - metadata (validations, callbacks, relations, scopes) copies down at
#     class definition
# Query-shape and inherited-validation assertions run without a database;
# behavior is gated behind the DB availability probe.
# ============================================================================
class StiUser < Model
  has_many "sti_posts"
  validates("email", {"presence": true})
  scope("by_key_desc", fn() { this.order("_key", "desc") })

  before_save("normalize_email")

  def normalize_email
    this.email = this.email.trim() unless this.email.nil?
    true
  end
end

class StiAdmin < StiUser
  def badge
    "admin"
  end
end

class StiSuperAdmin < StiAdmin
  def badge
    "super"
  end
end

class StiPost < Model
  belongs_to "sti_user"
end

# Detect DB availability
let __db_available = false
try
  let __probe = StiUser.create({"email": "probe@x.co"})
  if !__probe.nil? && !__probe._errors
    __db_available = true
    __probe.delete()
  end
catch e
  __db_available = false
end

describe("STI class wiring", fn() {
  test("subclass queries target the base collection with a type scope", fn() {
    let q = StiAdmin.where("email == @e", {"e": "x"}).to_query
    assert(q.includes?("FOR doc IN sti_users"))
    assert(q.includes?("doc.type IN [\"StiAdmin\", \"StiSuperAdmin\"]"))
  })

  test("the base class matches every row (no type filter)", fn() {
    let q = StiUser.where("email == @e", {"e": "x"}).to_query
    assert(q.includes?("FOR doc IN sti_users"))
    assert_not(q.includes?("doc.type IN"))
  })

  test("validations copy down to subclasses", fn() {
    let invalid = StiAdmin.create({})
    assert_not_null(invalid._errors)
    assert_eq(invalid._errors[0]["field"], "email")
  })

  test("scopes copy down to subclasses", fn() {
    let q = StiAdmin.by_key_desc.to_query
    assert(q.includes?("SORT doc._key DESC"))
    assert(q.includes?("doc.type IN"))
  })
})

describe("STI persistence and hydration", fn() {
  test("subclass creates stamp the discriminator in the base collection", fn() {
    if __db_available
      let admin = StiAdmin.create({"email": "a@x.co"})
      assert_eq(admin.type, "StiAdmin")

      # Visible through the base class, in the shared collection.
      let via_base = StiUser.find(admin._key)
      assert_eq(via_base.type, "StiAdmin")
      # ...and hydrated as the subclass.
      assert_eq(via_base.badge(), "admin")

      admin.delete()
    end
  })

  test("base queries return mixed rows hydrated per type", fn() {
    if __db_available
      let user = StiUser.create({"email": "u@x.co"})
      let admin = StiAdmin.create({"email": "b@x.co"})
      let super_admin = StiSuperAdmin.create({"email": "s@x.co"})

      assert_eq(StiUser.count(), 3)
      assert_eq(StiAdmin.count(), 2)       # includes descendants
      assert_eq(StiSuperAdmin.count(), 1)

      let badges = StiAdmin.all().map(fn(a) a.badge())
      assert(badges.includes?("admin"))
      assert(badges.includes?("super"))

      user.delete(); admin.delete(); super_admin.delete()
    end
  })

  test("a subclass find refuses rows outside its hierarchy", fn() {
    if __db_available
      let user = StiUser.create({"email": "plain@x.co"})

      let raised = false
      try
        StiAdmin.find(user._key)
      catch e
        raised = true
      end
      assert(raised)

      assert_null(StiAdmin.find_by("email", "plain@x.co"))
      assert_not_null(StiUser.find_by("email", "plain@x.co"))

      user.delete()
    end
  })

  test("save on an unpersisted subclass instance stamps type too", fn() {
    if __db_available
      let admin = StiAdmin.new({"email": "saved@x.co"})
      admin.save()
      assert_eq(admin.type, "StiAdmin")
      assert_eq(StiUser.find(admin._key).badge(), "admin")
      admin.delete()
    end
  })

  test("inherited callbacks run on subclass persists", fn() {
    if __db_available
      let admin = StiAdmin.create({"email": "  padded@x.co  "})
      assert_eq(admin.email, "padded@x.co")
      admin.delete()
    end
  })

  test("inherited relations use the base foreign key", fn() {
    if __db_available
      let admin = StiAdmin.create({"email": "author@x.co"})
      StiPost.create({"sti_user_id": admin._key, "title": "hello"})

      assert_eq(admin.sti_posts.count(), 1)

      StiPost.where("sti_user_id == @k", {"k": admin._key}).delete_all()
      admin.delete()
    end
  })

  test("subclass delete_all only removes its own hierarchy", fn() {
    if __db_available
      let user = StiUser.create({"email": "keep@x.co"})
      let admin = StiAdmin.create({"email": "drop@x.co"})

      StiAdmin.delete_all()

      assert_not_null(StiUser.find_by("email", "keep@x.co"))
      assert_null(StiUser.find_by("email", "drop@x.co"))

      user.delete()
    end
  })

  test("class-form update and delete refuse rows outside the hierarchy", fn() {
    if __db_available
      let user = StiUser.create({"email": "safe@x.co"})

      let update_result = StiAdmin.update(user._key, {"email": "hacked@x.co"})
      assert(str(update_result).includes?("Error"))
      assert_eq(StiUser.find(user._key).email, "safe@x.co")

      let delete_result = StiAdmin.delete(user._key)
      assert(str(delete_result).includes?("Error"))
      assert_not_null(StiUser.find_by("email", "safe@x.co"))

      user.delete()
    end
  })
})
