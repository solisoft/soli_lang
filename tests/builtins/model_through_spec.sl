# has_many through: traverses an intermediate relation as a chainable
# QueryBuilder. Read-only apart from `<<`: eager-loading and bulk writes raise.
# Query shapes and error paths need no database; live reads run on SoliDB.

class ThrUser < Model
  has_many("thr_memberships")
  has_many("thr_teams", through: "thr_memberships")
  has_many(
    "thr_employers",
    {"through": "thr_memberships", "source": "thr_company"}
  )
end

class ThrMembership < Model
  belongs_to("thr_user")
  belongs_to("thr_team")
  belongs_to("thr_company")
end

class ThrTeam < Model
end

class ThrCompany < Model
end

# Distant children: comments through posts (has_many source).
class ThrBlogUser < Model
  has_many("thr_posts")
  has_many(
    "thr_comments",
    {"through": "thr_posts", "source": "thr_comments"}
  )
end

class ThrPost < Model
  belongs_to("thr_blog_user")
  has_many("thr_comments")
end

class ThrComment < Model
  belongs_to("thr_post")
end

# Soft-deleting through model: the join subquery must skip deleted rows.
class ThrSdUser < Model
  has_many("thr_sd_memberships")
  has_many("thr_sd_groups", through: "thr_sd_memberships")
end

class ThrSdMembership < Model
  soft_delete
  belongs_to("thr_sd_user")
  belongs_to("thr_sd_group")
end

class ThrSdGroup < Model
end

# Error-path probes.
class ThrBroken < Model
  has_many("thr_ghost_things", through: "thr_ghosts")
end

class ThrNoSource < Model
  has_many("thr_orphan_widgets", through: "thr_carriers")
  has_many("thr_carriers")
end

class ThrCarrier < Model
  belongs_to("thr_no_source")
end

const TEAMS_QUERY = "FOR doc IN thr_teams FILTER doc._key IN " +
  "(FOR jt IN thr_memberships FILTER jt.thr_user_id == @__soli_through_fk RETURN jt.thr_team_id)"

# The SDBQL text of a builder, without the bind-variable dump that follows it.
def query_of(builder)
  builder.to_query.split(" | bind_vars:")[0]
end

def names_of(records)
  records.map { |record| record.name }
end

describe("through: query shape") do
  test("a belongs_to source selects targets whose key the join rows hold") do
    assert_eq(query_of(ThrUser.new({}).thr_teams), "#{TEAMS_QUERY} RETURN doc")
  end

  test("a chained where keeps both filters") do
    query = query_of(ThrUser.new({}).thr_teams.where("active == @a", {"a": true}))
    assert_eq(query, "#{TEAMS_QUERY} FILTER doc.active == @a RETURN doc")
  end

  test("source: picks the foreign key the join rows hold") do
    expected = "FOR doc IN thr_companies FILTER doc._key IN " +
      "(FOR jt IN thr_memberships FILTER jt.thr_user_id == @__soli_through_fk RETURN jt.thr_company_id) RETURN doc"
    assert_eq(query_of(ThrUser.new({}).thr_employers), expected)
  end

  test("a has_many source targets the distant children") do
    expected = "FOR doc IN thr_comments FILTER doc.thr_post_id IN " +
      "(FOR jt IN thr_posts FILTER jt.thr_blog_user_id == @__soli_through_fk RETURN jt._key) RETURN doc"
    assert_eq(query_of(ThrBlogUser.new({}).thr_comments), expected)
  end

  test("a soft-deleting through model skips deleted join rows") do
    query = query_of(ThrSdUser.new({}).thr_sd_groups)
    assert_contains(query, "FILTER jt.thr_sd_user_id == @__soli_through_fk AND jt.deleted_at == null")
  end

  test("aggregations carry the through filter") do
    assert_eq(query_of(ThrUser.new({}).thr_teams.sum("budget")), "#{TEAMS_QUERY} RETURN SUM(doc.budget)")
  end
end

describe("through: error paths") do
  test("an undeclared through relation names the relation and the model") do
    assert_raises("on ThrBroken: no relation \"thr_ghosts\" is declared") do
      ThrBroken.new({}).thr_ghost_things
    end
  end

  test("a missing source relation suggests source:") do
    message = assert_raises("ThrCarrier declares no relation \"thr_orphan_widget\"") do
      ThrNoSource.new({}).thr_orphan_widgets
    end
    assert_contains(message, "pick one with source:")
  end

  test("eager-loading a through relation is refused") do
    assert_raises("through: relations can't be eager-loaded or join-filtered yet") do
      ThrUser.includes("thr_teams")
    end
  end

  test("delete_all on a through relation is refused") do
    assert_raises("delete_all on a through: relation is not supported") do
      ThrUser.new({}).thr_teams.delete_all()
    end
  end

  test("update_all on a through relation is refused") do
    assert_raises("update_all on a through: relation is not supported") do
      ThrUser.new({}).thr_teams.update_all({"x": 1})
    end
  end
end

describe("through: live queries") do
  before_each() do
    requires_solidb()
  end

  after_each() do
    [ThrUser, ThrTeam, ThrMembership, ThrBlogUser, ThrComment, ThrSdUser, ThrSdGroup, ThrSdMembership].each do |model|
      model.delete_all()
    end
  end

  test("reads related records across the join") do
    user = ThrUser.create({"name": "u"})
    team_a = ThrTeam.create({"name": "Team A", "active": true})
    team_b = ThrTeam.create({"name": "Team B", "active": false})
    ThrTeam.create({"name": "Team C", "active": true})
    ThrMembership.create({"thr_user_id": user._key, "thr_team_id": team_a._key})
    ThrMembership.create({"thr_user_id": user._key, "thr_team_id": team_b._key})

    assert_eq(user.thr_teams.count, 2)
    assert_eq(user.thr_teams.exists.first, true)
    assert_eq(user.thr_teams.where("active == @a", {"a": true}).count, 1)
    assert_eq(names_of(user.thr_teams.order("name", "asc").all), ["Team A", "Team B"])
    assert_eq(user.thr_teams.where("name == @n", {"n": "Team C"}).count, 0)
  end

  test("another owner's join rows stay invisible") do
    owner = ThrUser.create({"name": "owner"})
    stranger = ThrUser.create({"name": "stranger"})
    team = ThrTeam.create({"name": "Owned"})
    ThrMembership.create({"thr_user_id": owner._key, "thr_team_id": team._key})
    assert_eq(stranger.thr_teams.count, 0)
    assert_null(stranger.thr_teams.first)
  end

  test("exists respects the through filter") do
    pending("bug: executing .exists rebuilds the query from `FOR doc IN <collection>` and drops the through: filter")
    owner = ThrUser.create({"name": "owner"})
    stranger = ThrUser.create({"name": "stranger"})
    team = ThrTeam.create({"name": "Owned"})
    ThrMembership.create({"thr_user_id": owner._key, "thr_team_id": team._key})
    assert_eq(stranger.thr_teams.exists.first, false)
  end

  test("source: reads through the other foreign key") do
    user = ThrUser.create({"name": "employee"})
    company = ThrCompany.create({"name": "Acme"})
    ThrMembership.create({"thr_user_id": user._key, "thr_company_id": company._key})
    assert_eq(names_of(user.thr_employers.all), ["Acme"])
    ThrCompany.delete_all()
  end

  test("a has_many source reads the distant children") do
    blog_user = ThrBlogUser.create({"name": "writer"})
    post = ThrPost.create({"thr_blog_user_id": blog_user._key})
    ThrComment.create({"thr_post_id": post._key, "name": "first"})
    ThrComment.create({"thr_post_id": "someone-else", "name": "stray"})
    assert_eq(names_of(blog_user.thr_comments.all), ["first"])
    ThrPost.delete_all()
  end

  test("an unpersisted owner sees no rows") do
    ThrTeam.create({"name": "Loose"})
    assert_eq(ThrUser.new({}).thr_teams.count, 0)
  end

  test("<< creates the join record from an instance or a key") do
    user = ThrUser.create({"name": "pusher"})
    team = ThrTeam.create({"name": "Pushed Team"})
    user.thr_teams << team
    assert_eq(user.thr_teams.count, 1)
    membership = ThrMembership.first_by("thr_user_id", user._key)
    assert_eq(membership.thr_team_id, team._key)

    team_two = ThrTeam.create({"name": "Keyed Team"})
    user.thr_teams << team_two._key
    assert_eq(names_of(user.thr_teams.order("name").all), ["Keyed Team", "Pushed Team"])
  end

  test("<< on an unpersisted owner is refused") do
    team = ThrTeam.create({"name": "Orphaned"})
    assert_raises("cannot push to \"thr_teams\": save the owner record first") do
      ThrUser.new({}).thr_teams << team
    end
    assert_eq(ThrMembership.count, 0)
  end

  test("<< on a has_many-source through is refused") do
    blog_user = ThrBlogUser.create({"name": "writer"})
    assert_raises("its through source is a has_many") do
      blog_user.thr_comments << ThrComment.new({})
    end
  end

  test("soft-deleted join rows drop out until restored") do
    user = ThrSdUser.create({"name": "sd"})
    group = ThrSdGroup.create({"name": "G"})
    membership = ThrSdMembership.create({"thr_sd_user_id": user._key, "thr_sd_group_id": group._key})

    assert_eq(user.thr_sd_groups.count, 1)
    membership.delete
    assert_eq(user.thr_sd_groups.count, 0)
    membership.restore
    assert_eq(user.thr_sd_groups.count, 1)
  end
end
