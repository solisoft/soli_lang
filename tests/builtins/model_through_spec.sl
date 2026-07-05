# ============================================================================
# has_many through: — traverse an intermediate relation as a chainable
# QueryBuilder. Read-only in v1: eager-loading and bulk writes raise.
# Query-shape assertions run without a database via .to_query; behavior
# assertions are gated behind the DB availability probe.
# ============================================================================
class ThrUser < Model
  has_many "thr_memberships"
  has_many "thr_teams", through: "thr_memberships"
  has_many("thr_employers", {"through": "thr_memberships", "source": "thr_company"})
end

class ThrMembership < Model
  belongs_to "thr_user"
  belongs_to "thr_team"
  belongs_to "thr_company"
end

class ThrTeam < Model
end

class ThrCompany < Model
end

# Distant children: comments through posts (has_many source).
class ThrBlogUser < Model
  has_many "thr_posts"
  has_many("thr_comments", {"through": "thr_posts", "source": "thr_comments"})
end

class ThrPost < Model
  belongs_to "thr_blog_user"
  has_many "thr_comments"
end

class ThrComment < Model
  belongs_to "thr_post"
end

# Soft-deleting through model: the join subquery must skip deleted rows.
class ThrSdUser < Model
  has_many "thr_sd_memberships"
  has_many "thr_sd_groups", through: "thr_sd_memberships"
end

class ThrSdMembership < Model
  soft_delete
  belongs_to "thr_sd_user"
  belongs_to "thr_sd_group"
end

class ThrSdGroup < Model
end

# Error-path probes.
class ThrBroken < Model
  has_many "thr_ghost_things", through: "thr_ghosts"
end

class ThrNoSource < Model
  has_many "thr_orphan_widgets", through: "thr_carriers"
  has_many "thr_carriers"
end

class ThrCarrier < Model
  belongs_to "thr_no_source"
end

# Detect DB availability
let __db_available = false
try
  let __probe = ThrUser.create({"name": "__probe__"})
  if !__probe.nil? && !__probe._errors
    __db_available = true
    __probe.delete()
  end
catch e
  __db_available = false
end

describe("through: query shape", fn() {
  test("belongs_to source emits the membership subquery", fn() {
    let q = ThrUser.new({}).thr_teams.to_query
    assert(q.includes?("FOR doc IN thr_teams"))
    assert(q.includes?("doc._key IN (FOR jt IN thr_memberships"))
    assert(q.includes?("jt.thr_user_id == @__soli_through_fk"))
    assert(q.includes?("RETURN jt.thr_team_id)"))
  })

  test("chained where keeps both filters", fn() {
    let q = ThrUser.new({}).thr_teams.where("active == @a", {"a": true}).to_query
    assert(q.includes?("doc._key IN (FOR jt IN thr_memberships"))
    assert(q.includes?("doc.active == @a"))
  })

  test("source: override changes the selected foreign key", fn() {
    let q = ThrUser.new({}).thr_employers.to_query
    assert(q.includes?("FOR doc IN thr_companies"))
    assert(q.includes?("RETURN jt.thr_company_id)"))
  })

  test("has_many source targets the distant children", fn() {
    let q = ThrBlogUser.new({}).thr_comments.to_query
    assert(q.includes?("FOR doc IN thr_comments"))
    assert(q.includes?("doc.thr_post_id IN (FOR jt IN thr_posts"))
    assert(q.includes?("jt.thr_blog_user_id == @__soli_through_fk"))
    assert(q.includes?("RETURN jt._key)"))
  })

  test("a soft-deleting through model guards the join rows", fn() {
    let q = ThrSdUser.new({}).thr_sd_groups.to_query
    assert(q.includes?("jt.deleted_at == null"))
  })

  test("count and aggregations carry the through filter", fn() {
    let q = ThrUser.new({}).thr_teams.sum("budget").to_query
    assert(q.includes?("doc._key IN (FOR jt IN thr_memberships"))
  })
})

describe("through: error paths", fn() {
  test("an undeclared through relation raises with both names", fn() {
    let raised = false
    try
      ThrBroken.new({}).thr_ghost_things
    catch e
      raised = true
      assert(str(e).includes?("thr_ghosts"))
      assert(str(e).includes?("ThrBroken"))
    end
    assert(raised)
  })

  test("a missing source relation suggests source:", fn() {
    let raised = false
    try
      ThrNoSource.new({}).thr_orphan_widgets
    catch e
      raised = true
      assert(str(e).includes?("source:"))
    end
    assert(raised)
  })

  test("eager-loading a through relation raises", fn() {
    let raised = false
    try
      ThrUser.includes("thr_teams")
    catch e
      raised = true
      assert(str(e).includes?("through"))
    end
    assert(raised)
  })

  test("delete_all on a through relation raises", fn() {
    let raised = false
    try
      ThrUser.new({}).thr_teams.delete_all()
    catch e
      raised = true
      assert(str(e).includes?("through"))
    end
    assert(raised)
  })

  test("update_all on a through relation raises", fn() {
    let raised = false
    try
      ThrUser.new({}).thr_teams.update_all({"x": 1})
    catch e
      raised = true
      assert(str(e).includes?("through"))
    end
    assert(raised)
  })
})

describe("through: live queries", fn() {
  test("reads related records across the join", fn() {
    if __db_available
      let user = ThrUser.create({"name": "u"})
      let team_a = ThrTeam.create({"name": "Team A", "active": true})
      let team_b = ThrTeam.create({"name": "Team B", "active": false})
      let team_c = ThrTeam.create({"name": "Team C", "active": true})
      let m1 = ThrMembership.create({"thr_user_id": user._key, "thr_team_id": team_a._key})
      let m2 = ThrMembership.create({"thr_user_id": user._key, "thr_team_id": team_b._key})

      assert_eq(user.thr_teams.count(), 2)
      assert_eq(user.thr_teams.exists().first, true)
      assert_eq(user.thr_teams.where("active == @a", {"a": true}).count(), 1)
      let names = user.thr_teams.order("name", "asc").all().map(fn(t) t.name)
      assert_eq(names, ["Team A", "Team B"])

      # Unrelated team stays invisible.
      assert_eq(user.thr_teams.where("name == @n", {"n": "Team C"}).count(), 0)

      m1.delete(); m2.delete()
      team_a.delete(); team_b.delete(); team_c.delete()
      user.delete()
    end
  })

  test("an unpersisted owner sees no rows", fn() {
    if __db_available
      let team = ThrTeam.create({"name": "Loose"})
      assert_eq(ThrUser.new({}).thr_teams.count(), 0)
      team.delete()
    end
  })

  test("soft-deleted join rows drop out of the association", fn() {
    if __db_available
      let user = ThrSdUser.create({"name": "sd"})
      let group = ThrSdGroup.create({"name": "G"})
      let membership = ThrSdMembership.create({"thr_sd_user_id": user._key, "thr_sd_group_id": group._key})

      assert_eq(user.thr_sd_groups.count(), 1)
      membership.delete()  # soft delete
      assert_eq(user.thr_sd_groups.count(), 0)

      membership.restore()
      assert_eq(user.thr_sd_groups.count(), 1)

      # Clean up (hard-remove the soft-deleted join row via bulk).
      ThrSdMembership.where("_key == @k", {"k": membership._key}).delete_all()
      group.delete()
      user.delete()
    end
  })
})
