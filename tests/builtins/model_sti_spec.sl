# Single-collection inheritance (STI): a model inheriting from another model
# shares its base's collection with a `type` discriminator. Subclass writes
# stamp `type`, rows hydrate as their stored type, subclass queries are scoped
# to their hierarchy, and validations/callbacks/relations/scopes copy down.

class StiUser < Model
  has_many("sti_posts")
  validates("email", {"presence": true})
  scope("by_key_desc", fn() { this.order("_key", "desc") })
  before_save("normalize_email")

  def normalize_email
    @email = @email.trim unless @email.nil?
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
  belongs_to("sti_user")
end

const ADMIN_SCOPE = "FILTER doc.type IN [\"StiAdmin\", \"StiSuperAdmin\"]"

# The SDBQL text of a builder, without the bind-variable dump that follows it.
def query_of(builder)
  builder.to_query.split(" | bind_vars:")[0]
end

describe("STI class wiring") do
  test("subclass queries target the base collection with a type scope") do
    query = query_of(StiAdmin.where("email == @e", {"e": "x"}))
    assert_eq(query, "FOR doc IN sti_users #{ADMIN_SCOPE} FILTER doc.email == @e RETURN doc")
  end

  test("a leaf subclass scopes to its own type") do
    query = query_of(StiSuperAdmin.where("email == @e", {"e": "x"}))
    assert_contains(query, "FILTER doc.type IN [\"StiSuperAdmin\"]")
  end

  test("the base class matches every row") do
    query = query_of(StiUser.where("email == @e", {"e": "x"}))
    assert_eq(query, "FOR doc IN sti_users FILTER doc.email == @e RETURN doc")
  end

  test("validations copy down to subclasses") do
    invalid = StiAdmin.create({})
    assert_eq(invalid._errors, [{"field": "email", "message": "can't be blank"}])
    assert_null(invalid._key)
  end

  test("scopes copy down to subclasses") do
    assert_eq(query_of(StiAdmin.by_key_desc), "FOR doc IN sti_users #{ADMIN_SCOPE} SORT doc._key DESC RETURN doc")
  end
end

describe("STI persistence and hydration") do
  before_each() do
    requires_solidb()
  end

  after_each() do
    StiUser.delete_all()
    StiPost.delete_all()
  end

  test("a subclass create stamps the discriminator in the base collection") do
    admin = StiAdmin.create({"email": "a@x.co"})
    assert_eq(admin.type, "StiAdmin")
    via_base = StiUser.find(admin._key)
    assert_eq(via_base.type, "StiAdmin")
    assert(via_base.is_a?("StiAdmin"))
    assert_eq(via_base.badge, "admin")
  end

  test("a base create stores no discriminator") do
    user = StiUser.create({"email": "u@x.co"})
    assert_null(user.type)
    assert_null(StiUser.find(user._key).type)
  end

  test("base queries return mixed rows hydrated per type") do
    StiUser.create({"email": "u@x.co"})
    StiAdmin.create({"email": "b@x.co"})
    StiSuperAdmin.create({"email": "s@x.co"})

    assert_eq(StiUser.count, 3)
    assert_eq(StiAdmin.count, 2)
    assert_eq(StiSuperAdmin.count, 1)
    assert_eq(StiAdmin.order("email").all.map { |admin| admin.badge }, ["admin", "super"])
  end

  test("a subclass find refuses rows outside its hierarchy") do
    user = StiUser.create({"email": "plain@x.co"})
    assert_raises("StiAdmin with id '#{user._key}' not found") do
      StiAdmin.find(user._key)
    end
    assert_null(StiAdmin.find_by("email", "plain@x.co"))
    assert_null(StiAdmin.first_by("email", "plain@x.co"))
    assert_eq(StiUser.find_by("email", "plain@x.co")._key, user._key)
  end

  test("a subclass exists refuses rows outside its hierarchy") do
    pending("bug: executing .exists rebuilds the query from `FOR doc IN <collection>` and drops the STI type scope")
    StiUser.create({"email": "plain@x.co"})
    assert_eq(StiAdmin.where("email == @e", {"e": "plain@x.co"}).exists.first, false)
  end

  test("save() on an unpersisted subclass instance stamps type too") do
    admin = StiAdmin.new({"email": "saved@x.co"})
    assert_eq(admin.save(), true)
    assert_eq(admin.type, "StiAdmin")
    assert_eq(StiUser.find(admin._key).badge, "admin")
  end

  test("inherited callbacks run on subclass persists") do
    admin = StiAdmin.create({"email": "  padded@x.co  "})
    assert_eq(admin.email, "padded@x.co")
    assert_eq(StiUser.find(admin._key).email, "padded@x.co")
  end

  test("inherited relations use the base foreign key") do
    admin = StiAdmin.create({"email": "author@x.co"})
    StiPost.create({"sti_user_id": admin._key, "title": "hello"})
    StiPost.create({"sti_user_id": "someone-else", "title": "other"})
    assert_eq(admin.sti_posts.count, 1)
    assert_eq(admin.sti_posts.first.title, "hello")
  end

  test("a subclass delete_all only removes its own hierarchy") do
    StiUser.create({"email": "keep@x.co"})
    StiAdmin.create({"email": "drop@x.co"})
    StiSuperAdmin.create({"email": "drop-too@x.co"})

    StiAdmin.delete_all()

    assert_eq(StiUser.all.map { |user| user.email }, ["keep@x.co"])
  end

  test("class-form update and delete refuse rows outside the hierarchy") do
    user = StiUser.create({"email": "safe@x.co"})
    not_found = "Error: StiAdmin with id '#{user._key}' not found"

    assert_eq(StiAdmin.update(user._key, {"email": "hacked@x.co"}), not_found)
    assert_eq(StiUser.find(user._key).email, "safe@x.co")

    assert_eq(StiAdmin.delete(user._key), not_found)
    assert_eq(StiUser.count, 1)
  end
end
